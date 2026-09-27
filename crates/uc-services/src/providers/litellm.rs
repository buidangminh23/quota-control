//! LiteLLM: the budgets and spend a LiteLLM proxy reports for one of its keys, and optionally the
//! key owner's model activity.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key, the proxy's server address and
//! the optional model-activity setting are saved in Quota Control. The address must use HTTPS, or
//! plain HTTP to this computer or a private network only, and a trailing `/v1` is dropped. Every
//! request is a read-only `GET` with the key as a bearer token. A refresh asks `/key/info` which
//! user and team the key belongs to, keeping that answer for 12 hours, then reads the user's
//! budget, spend and team budget from `/user/info`, or a team key's budget and spend from
//! `/team/info`. When `/key/info` answers 401, 403 or 404, it reads this month's spend from
//! `/key/spend/report` instead, and from `/user/spend/report` when that answers the same. With
//! model activity turned on it also reads the user's last 30 days of `/user/daily/activity`, at
//! most two pages, so a refresh sends at most four requests; activity that needs more pages is left
//! out rather than shown incomplete. Error bodies from the proxy are never shown.

use std::collections::{BTreeMap, HashSet};

use async_trait::async_trait;
use chrono::Duration;
use serde_json::Value;
use uc_core::{
    HttpRequest, MetricKind, MetricLine, MetricValue, Provider, SimpleProviderError,
    WidgetDescriptor,
};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{endpoint, http, lines, value};

pub(crate) struct LiteLLM;

const NAME: &str = "LiteLLM";

#[async_trait]
impl Service for LiteLLM {
    fn id(&self) -> &'static str {
        "litellm"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &[],
            url: "",
            fields: &[
                ("baseUrl", "Server address"),
                ("modelUsageEnabled", "Show model activity (true or false)"),
            ],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let mut descriptors = vec![];
        for (s, title) in [("personal", "Personal Budget"), ("team", "Team Budget")] {
            descriptors.push(
                WidgetDescriptor::bounded_dollars(
                    format!("{}.{s}", provider.id),
                    provider,
                    title,
                    None,
                    100.0,
                    None,
                    None,
                )
                .exporting_progress(s, "usd"),
            );
        }
        for (s, title) in [("spend", "Total Usage"), ("teamSpend", "Team Spend")] {
            descriptors.push(WidgetDescriptor::values(
                format!("{}.{s}", provider.id),
                provider,
                title,
                None,
                Some(MetricKind::Dollars),
                None,
                true,
                None,
                false,
            ));
        }
        descriptors
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("Enter the LiteLLM API key."))?;
        let base = endpoint::base_url(
            context.secret.str("/baseUrl"),
            None,
            endpoint::Policy::HttpsOrPrivateNetworkHttp,
            NAME,
        )?;
        let base = base.strip_suffix("/v1").unwrap_or(&base);
        let memo_key = "litellm.identity";
        let root = match context
            .memo
            .get(memo_key, context.now)
            .await
            .filter(|root| root["base"].as_str() == Some(base))
        {
            Some(root) => Some(root),
            None => request(context, key, &format!("{base}/key/info"), true).await?,
        };
        let Some(root) = root else {
            let end = context.now.format("%Y-%m-%d");
            let start = context.now.format("%Y-%m-01");
            let query = format!("?start_date={start}&end_date={end}");
            let report = match request(
                context,
                key,
                &format!("{base}/key/spend/report{query}"),
                true,
            )
            .await?
            {
                Some(report) => report,
                None => request(
                    context,
                    key,
                    &format!("{base}/user/spend/report{query}"),
                    false,
                )
                .await?
                .ok_or_else(|| http::decoding(NAME))?,
            };
            let rows = report
                .as_array()
                .filter(|entries| !entries.is_empty())
                .ok_or_else(|| http::decoding(NAME))?;
            let mut sum = 0.0;
            for row in rows {
                sum += number(row, "total_cost")?
                    .filter(|cost| *cost >= 0.0)
                    .ok_or_else(|| http::decoding(NAME))?;
            }
            if !sum.is_finite() {
                return Err(http::decoding(NAME));
            }
            return Ok(Reading::new(
                None,
                vec![lines::dollar_value("Total Usage", sum)],
            ));
        };
        let info = object(&root["info"])?;
        let user = value::text(info, "/user_id");
        let team = value::text(info, "/team_id");
        if user.is_none() && team.is_none() {
            return Err(http::decoding(NAME));
        }
        context
            .memo
            .put(
                memo_key,
                serde_json::json!({"base":base,"info":{"user_id":user,"team_id":team}}),
                Some(context.now + Duration::hours(12)),
            )
            .await;
        let mut out = vec![];
        if let Some(user) = user {
            let url = query(base, "user/info", &[("user_id", user)]);
            let response = request(context, key, &url, false)
                .await?
                .ok_or_else(|| http::decoding(NAME))?;
            let personal = object(&response["user_info"])?;
            let returned =
                value::text(personal, "/user_id").or_else(|| value::text(&response, "/user_id"));
            if returned.is_some_and(|id| id != user) {
                return Err(http::decoding(NAME));
            }
            let personal = budget(personal, "Personal Budget")?;
            if let Some(line) = personal.0 {
                out.push(line);
            }
            let teams = if response["teams"].is_null() {
                &[][..]
            } else {
                response["teams"]
                    .as_array()
                    .ok_or_else(|| http::decoding(NAME))?
                    .as_slice()
            };
            let mut selected = None;
            for listed in teams {
                let listed = object(listed)?;
                let id = value::text(listed, "/team_id").ok_or_else(|| http::decoding(NAME))?;
                let parsed = budget(listed, "Team Budget")?;
                if Some(id) == team {
                    selected = Some(parsed);
                }
            }
            if let Some((Some(line), _)) = &selected {
                out.push(line.clone());
            }
            out.push(lines::dollar_value("Total Usage", personal.1));
            if let Some((None, spend)) = selected {
                out.push(lines::dollar_value("Team Spend", spend));
            }
            if context.secret.str("/modelUsageEnabled") == Some("true")
                && let Ok(models) = activity(context, key, base, user).await
            {
                out.extend(models);
            }
        } else if let Some(team) = team {
            let response = request(
                context,
                key,
                &query(base, "team/info", &[("team_id", team)]),
                false,
            )
            .await?
            .ok_or_else(|| http::decoding(NAME))?;
            let info = object(&response["team_info"])?;
            let returned =
                value::text(info, "/team_id").or_else(|| value::text(&response, "/team_id"));
            if returned.is_some_and(|id| id != team) {
                return Err(http::decoding(NAME));
            }
            let (line, spend) = budget(info, "Team Budget")?;
            if let Some(line) = line {
                out.push(line);
            }
            out.push(lines::dollar_value("Total Usage", spend));
        }
        Ok(Reading::new(None, out))
    }
}

/// The value itself when it is a JSON object; anything else is an answer this version cannot read.
fn object(candidate: &Value) -> Result<&Value, SimpleProviderError> {
    if candidate.is_object() {
        Ok(candidate)
    } else {
        Err(http::decoding(NAME))
    }
}

/// The finite number in `record[field]`, `None` when that field is null or absent; any other value
/// cannot be read.
fn number(record: &Value, field: &str) -> Result<Option<f64>, SimpleProviderError> {
    if record[field].is_null() {
        return Ok(None);
    }
    record[field]
        .as_f64()
        .filter(|amount| amount.is_finite())
        .map(Some)
        .ok_or_else(|| http::decoding(NAME))
}

/// A user's or team's budget meter when it has a maximum budget, and its spend.
fn budget(record: &Value, title: &str) -> Result<(Option<MetricLine>, f64), SimpleProviderError> {
    object(record)?;
    let used = number(record, "spend")?.unwrap_or(0.0);
    let limit = number(record, "max_budget")?.unwrap_or(0.0);
    Ok((
        (limit > 0.0).then(|| {
            lines::dollars(
                title,
                used,
                limit,
                value::time(record, "/budget_reset_at"),
                None,
            )
        }),
        used,
    ))
}

/// `base/path` with `params` form-encoded as its query.
fn query(base: &str, path: &str, params: &[(&str, &str)]) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params.iter().copied())
        .finish();
    format!("{base}/{path}?{query}")
}

/// A GET of `url` with the key as a bearer token, answered as JSON. With `optional`, a 401, 403 or
/// 404 answer is `None` so the caller can try another endpoint.
async fn request(
    context: &FetchContext<'_>,
    key: &str,
    url: &str,
    optional: bool,
) -> Result<Option<Value>, SimpleProviderError> {
    let response = http::send(
        context.http,
        HttpRequest::get(url)
            .header("Authorization", format!("Bearer {key}"))
            .header("Accept", "application/json"),
        NAME,
    )
    .await?;
    if optional && [401, 403, 404].contains(&response.status) {
        return Ok(None);
    }
    if !response.is_success() {
        return Err(http::status_error(&response, NAME));
    }
    Ok(Some(http::parse(&response, NAME)?))
}

/// The user's token and request totals per model over the last 30 days, from at most two pages of
/// daily activity; activity that needs more pages is not available.
async fn activity(
    context: &FetchContext<'_>,
    key: &str,
    base: &str,
    user: &str,
) -> Result<Vec<MetricLine>, SimpleProviderError> {
    let end = context.now.format("%Y-%m-%d").to_string();
    let start = (context.now - Duration::days(29))
        .format("%Y-%m-%d")
        .to_string();
    let mut totals: BTreeMap<String, [f64; 4]> = BTreeMap::new();
    let mut days = HashSet::new();
    let count = |record: &Value, field: &str| {
        number(record, field)?
            .filter(|amount| {
                *amount >= 0.0 && amount.fract() == 0.0 && *amount <= 9007199254740991.0
            })
            .ok_or_else(|| http::decoding(NAME))
    };
    for page in 1..=2 {
        let url = query(
            base,
            "user/daily/activity",
            &[
                ("user_id", user),
                ("start_date", &start),
                ("end_date", &end),
                ("page", &page.to_string()),
                ("page_size", "1000"),
            ],
        );
        let root = request(context, key, &url, false)
            .await?
            .ok_or_else(|| http::decoding(NAME))?;
        let rows = root["results"]
            .as_array()
            .filter(|results| results.len() <= 31)
            .ok_or_else(|| http::decoding(NAME))?;
        for row in rows {
            let day = value::text(row, "/date")
                .filter(|date| date.len() == 10 && *date >= start.as_str() && *date <= end.as_str())
                .ok_or_else(|| http::decoding(NAME))?;
            if chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").is_err()
                || !days.insert(day.to_string())
            {
                return Err(http::decoding(NAME));
            }
            let models = row
                .pointer("/breakdown/models")
                .and_then(Value::as_object)
                .ok_or_else(|| http::decoding(NAME))?;
            for (name, entry) in models {
                if name.trim().is_empty()
                    || name.chars().count() > 120
                    || name.chars().any(char::is_control)
                {
                    return Err(http::decoding(NAME));
                }
                let metrics = if entry["metrics"].is_null() {
                    object(entry)?
                } else {
                    object(&entry["metrics"])?
                };
                let sums = totals.entry(name.clone()).or_insert([0.0; 4]);
                for (index, field) in [
                    "prompt_tokens",
                    "completion_tokens",
                    "total_tokens",
                    "api_requests",
                ]
                .iter()
                .enumerate()
                {
                    sums[index] += count(metrics, field)?;
                    if !sums[index].is_finite() || sums[index] > 9007199254740991.0 {
                        return Err(http::decoding(NAME));
                    }
                }
                if totals.len() > 1000 {
                    return Err(http::decoding(NAME));
                }
            }
        }
        let meta = &root["metadata"];
        if !meta.is_null() {
            object(meta)?;
        }
        if !meta["page"].is_null() && count(meta, "page")? != page as f64 {
            return Err(http::decoding(NAME));
        }
        if !meta["has_more"].is_null() && !meta["has_more"].is_boolean() {
            return Err(http::decoding(NAME));
        }
        let pages = if meta["total_pages"].is_null() {
            1.0
        } else {
            count(meta, "total_pages")?
        };
        if meta["has_more"] != true && page as f64 >= pages {
            return Ok(totals
                .into_iter()
                .map(|(name, [input, output, total, requests])| {
                    let name = if [
                        "Personal Budget",
                        "Team Budget",
                        "Total Usage",
                        "Team Spend",
                    ]
                    .contains(&name.as_str())
                    {
                        format!("Model: {name}")
                    } else {
                        name
                    };
                    lines::values(
                        &name,
                        vec![
                            MetricValue::count(total, "tokens"),
                            MetricValue::count(requests, "requests"),
                            MetricValue::count(input, "tokens").with_label("Input"),
                            MetricValue::count(output, "tokens").with_label("Output"),
                        ],
                    )
                })
                .collect());
        }
    }
    Err(http::not_available("LiteLLM model activity is incomplete."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::{DateTime, Utc};
    use serde_json::json;
    use uc_core::ErrorCategory;

    const USER_INFO: &str = r#"{"user_info":{"user_id":"u1","spend":212.35,"max_budget":300},"teams":[{"team_id":"other","spend":1,"max_budget":2},{"team_id":"t1","spend":215.32,"max_budget":1000,"budget_reset_at":"2026-10-01T00:00:00Z"}]}"#;
    const DAILY_ACTIVITY: &str = r#"{"results":[{"date":"2026-09-27","breakdown":{"models":{"gpt-4o":{"metrics":{"prompt_tokens":100,"completion_tokens":20,"total_tokens":120,"api_requests":3}}}}}],"metadata":{"page":1,"total_pages":1}}"#;
    const USER_SPEND_REPORT: &str =
        "https://gateway.example/user/spend/report?start_date=2026-09-01&end_date=2026-09-27";

    fn now() -> DateTime<Utc> {
        "2026-09-27T12:00:00Z".parse().unwrap()
    }

    fn secret() -> Value {
        json!({"apiKey":"fixture","baseUrl":"https://gateway.example/v1/","modelUsageEnabled":"true"})
    }

    #[tokio::test]
    async fn a_user_key_shows_both_budgets_its_spend_and_its_model_activity() {
        let http = Scripted::new()
            .on(
                "GET",
                "https://gateway.example/key/info",
                200,
                r#"{"info":{"user_id":"u1","team_id":"t1"}}"#,
            )
            .on(
                "GET",
                "https://gateway.example/user/info?user_id=u1",
                200,
                USER_INFO,
            )
            .on(
                "GET",
                "https://gateway.example/user/daily/activity?",
                200,
                DAILY_ACTIVITY,
            );
        let scope = context_at(&http, secret(), now());
        let reading = LiteLLM.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan, None);
        assert_eq!(
            reading.lines,
            vec![
                lines::dollars("Personal Budget", 212.35, 300.0, None, None),
                lines::dollars(
                    "Team Budget",
                    215.32,
                    1000.0,
                    Some("2026-10-01T00:00:00Z".parse().unwrap()),
                    None
                ),
                lines::dollar_value("Total Usage", 212.35),
                lines::values(
                    "gpt-4o",
                    vec![
                        MetricValue::count(120.0, "tokens"),
                        MetricValue::count(3.0, "requests"),
                        MetricValue::count(100.0, "tokens").with_label("Input"),
                        MetricValue::count(20.0, "tokens").with_label("Output")
                    ]
                )
            ]
        );
        assert_eq!(http.requests().len(), 3);
        assert_eq!(
            header(&http.requests()[0], "Authorization"),
            Some("Bearer fixture")
        );
        assert!(http.requests()[2].url.contains("start_date=2026-08-29"));
    }

    #[tokio::test]
    async fn refused_key_info_falls_back_to_spend_reports_and_team_errors_keep_their_category() {
        let http = Scripted::new()
            .on("GET", "https://gateway.example/key/info", 403, "secret")
            .on(
                "GET",
                "https://gateway.example/key/spend/report?",
                404,
                "secret",
            )
            .on(
                "GET",
                USER_SPEND_REPORT,
                200,
                r#"[{"total_cost":1.5},{"total_cost":2}]"#,
            );
        let scope = context_at(&http, secret(), now());
        assert_eq!(
            LiteLLM.fetch(&scope.context()).await.unwrap().lines,
            vec![lines::dollar_value("Total Usage", 3.5)]
        );
        assert_eq!(http.requests().len(), 3);
        for (status, category) in [
            (401, ErrorCategory::AuthExpired),
            (429, ErrorCategory::RateLimited),
        ] {
            let http = Scripted::new()
                .on(
                    "GET",
                    "https://gateway.example/key/info",
                    200,
                    r#"{"info":{"team_id":"t1"}}"#,
                )
                .on(
                    "GET",
                    "https://gateway.example/team/info?",
                    status,
                    "secret",
                );
            let scope = context_at(&http, secret(), now());
            let error = LiteLLM.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, category);
            assert!(!error.message.contains("secret"));
        }
    }

    #[tokio::test]
    async fn optional_history_cannot_replace_budgets() {
        let http = Scripted::new()
            .on(
                "GET",
                "https://gateway.example/key/info",
                200,
                r#"{"info":{"user_id":"u1"}}"#,
            )
            .on(
                "GET",
                "https://gateway.example/user/info?",
                200,
                r#"{"user_info":{"spend":5}}"#,
            )
            .on(
                "GET",
                "https://gateway.example/user/daily/activity?",
                200,
                r#"{"results":[],"metadata":{"has_more":true}}"#,
            )
            .on(
                "GET",
                "https://gateway.example/user/daily/activity?",
                200,
                r#"{"results":[],"metadata":{"has_more":true}}"#,
            );
        let scope = context_at(&http, secret(), now());
        assert_eq!(
            LiteLLM.fetch(&scope.context()).await.unwrap().lines,
            vec![lines::dollar_value("Total Usage", 5.0)]
        );
        assert_eq!(http.requests().len(), 4);
    }

    #[tokio::test]
    async fn identity_cache_avoids_repeated_lookup() {
        let http = Scripted::new()
            .on(
                "GET",
                "https://gateway.example/key/info",
                200,
                r#"{"info":{"team_id":"t1"}}"#,
            )
            .on(
                "GET",
                "https://gateway.example/team/info?",
                200,
                r#"{"team_info":{"team_id":"t1","spend":5}}"#,
            )
            .on(
                "GET",
                "https://gateway.example/team/info?",
                200,
                r#"{"team_info":{"team_id":"t1","spend":6}}"#,
            );
        let scope = context_at(&http, secret(), now());
        assert_eq!(
            LiteLLM.fetch(&scope.context()).await.unwrap().lines,
            vec![lines::dollar_value("Total Usage", 5.0)]
        );
        assert_eq!(
            LiteLLM.fetch(&scope.context()).await.unwrap().lines,
            vec![lines::dollar_value("Total Usage", 6.0)]
        );
        assert_eq!(http.requests().len(), 3);
    }

    #[tokio::test]
    async fn mismatched_identity_is_rejected() {
        let http = Scripted::new()
            .on(
                "GET",
                "https://gateway.example/key/info",
                200,
                r#"{"info":{"team_id":"t1"}}"#,
            )
            .on(
                "GET",
                "https://gateway.example/team/info?",
                200,
                r#"{"team_info":{"team_id":"other","spend":5}}"#,
            );
        let scope = context_at(&http, secret(), now());
        assert_eq!(
            LiteLLM.fetch(&scope.context()).await.unwrap_err().category,
            ErrorCategory::Decoding
        );
    }
}
