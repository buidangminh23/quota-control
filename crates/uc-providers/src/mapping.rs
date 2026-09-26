use chrono::{DateTime, Duration, Utc};
use serde_json::{Map, Value};
use uc_core::{
    ErrorCategory, HttpResponse, LimitResetCredit, LimitResetReply, MetricLine, MetricValue,
    ProgressFormat, SimpleProviderError,
};

use crate::ProviderKind;

const SESSION_MS: i64 = 18_000_000;
const WEEK_MS: i64 = 604_800_000;

pub struct MappedUsage {
    pub plan: Option<String>,
    pub lines: Vec<MetricLine>,
}

pub(crate) fn reset_credit_expiries(
    response: &HttpResponse,
) -> Result<Vec<DateTime<Utc>>, SimpleProviderError> {
    let body: Value = response.json().map_err(|_| invalid())?;
    let credits = body["credits"].as_array().ok_or_else(invalid)?;
    let mut expiries = Vec::new();
    for credit in credits {
        if credit["status"].as_str().ok_or_else(invalid)? == "available" {
            expiries.push(timestamp(&credit["expires_at"])?.ok_or_else(invalid)?);
        }
    }
    Ok(expiries)
}

/// The credits Codex lists as spendable, soonest to expire first; one without an expiry goes last.
pub(crate) fn reset_credits(
    response: &HttpResponse,
) -> Result<Vec<LimitResetCredit>, SimpleProviderError> {
    let body: Value = response.json().map_err(|_| invalid())?;
    let credits = body["credits"].as_array().ok_or_else(invalid)?;
    let mut available = Vec::new();
    for credit in credits {
        if credit["status"].as_str().ok_or_else(invalid)? != "available" {
            continue;
        }
        let id = credit["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(invalid)?;
        available.push(LimitResetCredit {
            id: id.to_string(),
            expires_at: timestamp(&credit["expires_at"])?,
        });
    }
    available.sort_by_key(|credit| credit.expires_at.unwrap_or(DateTime::<Utc>::MAX_UTC));
    Ok(available)
}

/// Codex's answer to `consume`: its `code` and, on success, which limit came back.
pub(crate) fn limit_reset_reply(
    response: &HttpResponse,
) -> Result<LimitResetReply, SimpleProviderError> {
    let body: Value = response.json().map_err(|_| invalid())?;
    let code = body["code"]
        .as_str()
        .filter(|code| !code.is_empty())
        .ok_or_else(invalid)?;
    Ok(LimitResetReply {
        code: code.to_string(),
        reset_type: body["credit"]["reset_type"].as_str().map(str::to_string),
    })
}

fn invalid() -> SimpleProviderError {
    SimpleProviderError::new(
        ErrorCategory::Decoding,
        "The usage service returned an invalid response.",
    )
}

fn object(value: &Value) -> Result<Option<&Map<String, Value>>, SimpleProviderError> {
    if value.is_null() {
        return Ok(None);
    }
    value.as_object().map(Some).ok_or_else(invalid)
}

fn number(value: &Value) -> Result<Option<f64>, SimpleProviderError> {
    if value.is_null() {
        return Ok(None);
    }
    let parsed = value
        .as_f64()
        .or_else(|| value.as_str()?.parse::<f64>().ok());
    parsed
        .filter(|number| number.is_finite())
        .map(Some)
        .ok_or_else(invalid)
}

fn nonnegative(value: &Value) -> Result<Option<f64>, SimpleProviderError> {
    let number = number(value)?;
    if number.is_some_and(|number| number < 0.0) {
        return Err(invalid());
    }
    Ok(number)
}

fn timestamp(value: &Value) -> Result<Option<DateTime<Utc>>, SimpleProviderError> {
    if value.is_null() {
        return Ok(None);
    }
    if let Some(text) = value.as_str()
        && let Ok(date) = DateTime::parse_from_rfc3339(text)
    {
        return Ok(Some(date.with_timezone(&Utc)));
    }
    let number = number(value)?.ok_or_else(invalid)?;
    let millis = if number.abs() < 1e10 {
        number * 1000.0
    } else {
        number
    };
    DateTime::from_timestamp_millis(millis as i64)
        .map(Some)
        .ok_or_else(invalid)
}

pub fn title_case(raw: &str) -> String {
    raw.split(['_', ' '])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn map_response(
    kind: ProviderKind,
    response: &HttpResponse,
    now: DateTime<Utc>,
) -> Result<MappedUsage, SimpleProviderError> {
    if !response.is_success() {
        let (category, message) = match response.status {
            401 => (
                ErrorCategory::AuthExpired,
                "Session expired. Sign in again.".into(),
            ),
            403 => (
                ErrorCategory::AuthInvalid,
                "The login does not have permission to read usage. Sign in again.".into(),
            ),
            429 => (
                ErrorCategory::RateLimited,
                "Usage updates are rate limited. Try again later.".into(),
            ),
            status => (
                ErrorCategory::http(status),
                format!("Usage request failed (HTTP {status})."),
            ),
        };
        return Err(SimpleProviderError::new(category, message));
    }
    let body: Value = response.json().map_err(|_| invalid())?;
    if !body.is_object() {
        return Err(invalid());
    }
    let mut mapped = match kind {
        ProviderKind::Claude => map_claude(&body)?,
        ProviderKind::Codex => map_codex(&body, response, now)?,
    };
    MetricLine::append_no_data_if_needed(&mut mapped.lines);
    Ok(mapped)
}

fn map_claude(body: &Value) -> Result<MappedUsage, SimpleProviderError> {
    let mut lines = Vec::new();
    for (key, label, period) in [
        ("five_hour", "Session", SESSION_MS),
        ("seven_day", "Weekly", WEEK_MS),
        ("seven_day_sonnet", "Sonnet", WEEK_MS),
    ] {
        let window = &body[key];
        if object(window)?.is_none() {
            continue;
        }
        if let Some(used) = nonnegative(&window["utilization"])? {
            lines.push(
                MetricLine::progress(label, used, 100.0, ProgressFormat::Percent)
                    .period_ms(Some(period))
                    .resets_at(timestamp(&window["resets_at"])?)
                    .into(),
            );
        }
    }
    if let Some(limits) = body.get("limits").filter(|value| !value.is_null()) {
        for limit in limits.as_array().ok_or_else(invalid)? {
            if limit["kind"] == "weekly_scoped"
                && limit["scope"]["model"]["display_name"] == "Fable"
                && let Some(used) = nonnegative(&limit["percent"])?
            {
                lines.push(
                    MetricLine::progress("Fable", used, 100.0, ProgressFormat::Percent)
                        .period_ms(Some(WEEK_MS))
                        .resets_at(timestamp(&limit["resets_at"])?)
                        .into(),
                );
                break;
            }
        }
    }
    let extra = &body["extra_usage"];
    if object(extra)?.is_some() {
        match extra["is_enabled"].as_bool() {
            Some(false) => lines.push(MetricLine::badge("Extra usage spent", "Disabled").into()),
            Some(true) => {
                if let Some(used) = nonnegative(&extra["used_credits"])? {
                    let used = used / 100.0;
                    match nonnegative(&extra["monthly_limit"])? {
                        Some(limit) if limit > 0.0 => lines.push(
                            MetricLine::progress(
                                "Extra usage spent",
                                used,
                                limit / 100.0,
                                ProgressFormat::Dollars,
                            )
                            .into(),
                        ),
                        _ => lines.push(
                            MetricLine::values(
                                "Extra usage spent",
                                vec![MetricValue::dollars(used)],
                            )
                            .into(),
                        ),
                    }
                }
            }
            None if extra["is_enabled"].is_null() => {}
            None => return Err(invalid()),
        }
    }
    Ok(MappedUsage { plan: None, lines })
}

fn append_codex_windows(
    rate: &Value,
    labels: (&str, &str),
    header_percents: [Option<&str>; 2],
    now: DateTime<Utc>,
    lines: &mut Vec<MetricLine>,
) -> Result<(), SimpleProviderError> {
    object(rate)?;
    let mut candidates = Vec::new();
    for (index, key) in ["primary_window", "secondary_window"]
        .into_iter()
        .enumerate()
    {
        let window = &rate[key];
        object(window)?;
        let used = match nonnegative(&window["used_percent"])? {
            Some(used) => Some(used),
            None => match header_percents[index] {
                Some(raw) => nonnegative(&Value::String(raw.into()))?,
                None => None,
            },
        };
        let Some(used) = used else {
            continue;
        };
        let period = nonnegative(&window["limit_window_seconds"])?;
        if period.is_some_and(|period| period == 0.0 || period > i64::MAX as f64 / 1000.0) {
            return Err(invalid());
        }
        let period = period.map(|seconds| (seconds * 1000.0) as i64);
        let exact = match period {
            Some(SESSION_MS) => Some(0),
            Some(WEEK_MS) => Some(1),
            _ => None,
        };
        let resets = match timestamp(&window["reset_at"])? {
            Some(reset) => Some(reset),
            None => match nonnegative(&window["reset_after_seconds"])? {
                Some(seconds) if seconds <= i64::MAX as f64 / 1000.0 => {
                    now.checked_add_signed(Duration::milliseconds((seconds * 1000.0) as i64))
                }
                Some(_) => return Err(invalid()),
                None => None,
            },
        };
        candidates.push((index, exact, period, used, resets));
    }
    for (index, label) in [labels.0, labels.1].into_iter().enumerate() {
        let candidate = candidates
            .iter()
            .find(|entry| entry.1 == Some(index))
            .or_else(|| {
                candidates
                    .iter()
                    .find(|entry| entry.1.is_none() && entry.0 == index)
            });
        if let Some((_, _, period, used, reset)) = candidate {
            lines.push(
                MetricLine::progress(label, *used, 100.0, ProgressFormat::Percent)
                    .period_ms(Some(period.unwrap_or(if index == 0 {
                        SESSION_MS
                    } else {
                        WEEK_MS
                    })))
                    .resets_at(*reset)
                    .into(),
            );
        }
    }
    Ok(())
}

fn map_codex(
    body: &Value,
    response: &HttpResponse,
    now: DateTime<Utc>,
) -> Result<MappedUsage, SimpleProviderError> {
    let mut lines = Vec::new();
    append_codex_windows(
        &body["rate_limit"],
        ("Session", "Weekly"),
        [
            response.header("x-codex-primary-used-percent"),
            response.header("x-codex-secondary-used-percent"),
        ],
        now,
        &mut lines,
    )?;
    if let Some(additional) = body
        .get("additional_rate_limits")
        .filter(|value| !value.is_null())
    {
        for entry in additional.as_array().ok_or_else(invalid)? {
            if ["limit_name", "metered_feature"].into_iter().any(|key| {
                entry[key]
                    .as_str()
                    .is_some_and(|value| value.to_lowercase().contains("spark"))
            }) {
                append_codex_windows(
                    &entry["rate_limit"],
                    ("Spark", "Spark Weekly"),
                    [None, None],
                    now,
                    &mut lines,
                )?;
                break;
            }
        }
    }
    let credits = &body["credits"];
    object(credits)?;
    let balance = match number(&credits["balance"])? {
        Some(balance) => Some(balance),
        None if credits["has_credits"] == false => Some(0.0),
        None => match response.header("x-codex-credits-balance") {
            Some(raw) => number(&Value::String(raw.into()))?,
            None => None,
        },
    };
    if let Some(balance) = balance {
        let count = balance.floor().max(0.0);
        lines.push(
            MetricLine::values(
                "Credits",
                vec![
                    MetricValue::dollars(count * 0.04),
                    MetricValue::count(count, "credits"),
                ],
            )
            .into(),
        );
    }
    let resets = &body["rate_limit_reset_credits"];
    if object(resets)?.is_some()
        && let Some(count) = nonnegative(&resets["available_count"])?
    {
        lines.push(
            MetricLine::values(
                "Rate Limit Resets",
                vec![MetricValue::count(count.floor(), "available")],
            )
            .into(),
        );
    }
    let plan = match body.get("plan_type").filter(|value| !value.is_null()) {
        Some(value) => {
            let raw = value.as_str().ok_or_else(invalid)?.trim();
            if raw.is_empty() {
                None
            } else {
                Some(match raw.to_lowercase().as_str() {
                    "prolite" => "Pro 5x".into(),
                    "pro" => "Pro 20x".into(),
                    "self_serve_business_prolite" => "Business Premium".into(),
                    _ => title_case(raw),
                })
            }
        }
        None => None,
    };
    Ok(MappedUsage { plan, lines })
}
