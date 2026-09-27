//! CodeBuddy (Tencent Cloud's AI code assistant): an API key or a CodeBuddy platform token that
//! the user saves in Accounts or sets in `CODEBUDDY_API_KEY` / `CODEBUDDY_AUTH_TOKEN`, read the
//! same way on Windows, macOS and Linux. Nothing is read from this computer's files: CodeBuddy
//! Code keeps its login in the system credential store (Windows Credential Manager, the macOS
//! Keychain, the Linux Secret Service) under a name it does not document, so the card takes a key
//! instead of discovering a login.
//!
//! Each refresh posts `{}` to `https://www.codebuddy.ai/v2/billing/meter/get-user-resource` (the
//! international site). A key that site refuses (401 or 403), or a site that cannot be reached, is
//! tried at `https://copilot.tencent.com/v2/billing/meter/get-user-resource` (China), and the site
//! that accepted the key is asked first for the next 12 hours. The answer lists credit packages
//! under `data.Response.Data.Accounts`, and the two kinds are never merged: a package whose cycle
//! ends well before the package itself expires refills every cycle (the monthly base allowance,
//! daily promotional credits), so its `Cycle*` figures feed the Monthly or Daily meter and reset
//! when the cycle ends; any other package runs a single cycle and expires (gift, bonus and top-up
//! packs), so its lifetime `Capacity*` figures add up to the Bonus Credits row with each
//! package's expiry. Times without an offset are China Standard Time (UTC+8), as the billing
//! service writes them.

use async_trait::async_trait;
use chrono::{DateTime, Duration, FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde_json::{Value, json};
use uc_core::{
    HttpRequest, LimitResourceKind, LimitResourceSource, MetricKind, MetricLine, MetricValue,
    Provider, ProviderLink, SimpleProviderError, ValuesLine, WidgetDescriptor,
};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct CodeBuddy;

const NAME: &str = "CodeBuddy";
const USAGE_PATH: &str = "/v2/billing/meter/get-user-resource";
/// The client version CodeBuddy's own apps announce in the headers the usage endpoint receives.
const CLIENT_VERSION: &str = "2.108.1";

/// Memo key naming the site that last accepted this card's key.
const SITE_MEMO: &str = "codebuddy.site";
const SITE_MEMO_HOURS: i64 = 12;

const MONTHLY: &str = "Monthly";
const DAILY: &str = "Daily";
const BONUS: &str = "Bonus Credits";
const UNIT: &str = "credits";

/// A package still valid this long after its cycle ends refills every cycle.
const REFILL_GAP_HOURS: i64 = 48;
/// Refill cycles up to this long are daily allowances.
const DAILY_CYCLE_HOURS: i64 = 36;
/// The longest cycle trusted as a meter's window.
const LONGEST_CYCLE_DAYS: i64 = 400;

/// The two CodeBuddy sites; a key belongs to one of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Site {
    /// `www.codebuddy.ai`.
    International,
    /// `copilot.tencent.com`.
    China,
}

impl Site {
    fn key(self) -> &'static str {
        match self {
            Self::International => "international",
            Self::China => "china",
        }
    }

    fn origin(self) -> &'static str {
        match self {
            Self::International => "https://www.codebuddy.ai",
            Self::China => "https://copilot.tencent.com",
        }
    }

    fn other(self) -> Self {
        match self {
            Self::International => Self::China,
            Self::China => Self::International,
        }
    }

    /// The client CodeBuddy's own apps name on this site: the IDE internationally, the CLI in
    /// China.
    fn client(self) -> &'static str {
        match self {
            Self::International => "IDE",
            Self::China => "CLI",
        }
    }

    fn usage_request(self, key: &str) -> HttpRequest {
        let client = self.client();
        HttpRequest::post(format!("{}{USAGE_PATH}", self.origin()))
            .bearer(key)
            .header("Accept", "application/json")
            .header(
                "User-Agent",
                format!("{client}/{CLIENT_VERSION} CodeBuddy/{CLIENT_VERSION}"),
            )
            .header("X-Product", "SaaS")
            .header("X-IDE-Type", client)
            .header("X-IDE-Name", client)
            .header("X-Requested-With", "XMLHttpRequest")
            .header("X-CodeBuddy-Request", "1")
            .json_body(&json!({}))
    }
}

#[async_trait]
impl Service for CodeBuddy {
    fn id(&self) -> &'static str {
        "codebuddy"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![ProviderLink::new(
            "Plans",
            "https://www.codebuddy.ai/docs/ide/Account/pricing",
        )]
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["CODEBUDDY_API_KEY", "CODEBUDDY_AUTH_TOKEN"],
            url: "https://www.codebuddy.ai/profile/keys",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let id = |suffix: &str| format!("{}.{suffix}", provider.id);
        vec![
            WidgetDescriptor::bounded_count(
                id("monthly"),
                provider,
                MONTHLY,
                None,
                1000.0,
                UNIT,
                Some(lines::MONTH_MS),
            )
            .exporting_progress("monthly", UNIT),
            WidgetDescriptor::bounded_count(
                id("daily"),
                provider,
                DAILY,
                None,
                50.0,
                UNIT,
                Some(lines::DAY_MS),
            )
            .exporting_progress("daily", UNIT),
            WidgetDescriptor::values(
                id("bonus"),
                provider,
                BONUS,
                None,
                Some(MetricKind::Count),
                Some("left"),
                false,
                Some(UNIT),
                false,
            )
            .exporting_limit(
                "bonusCredits",
                LimitResourceKind::Balance,
                UNIT,
                LimitResourceSource::Value {
                    kind: MetricKind::Count,
                    label: Some(UNIT.into()),
                },
                false,
            ),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("No CodeBuddy API key is saved. Add one in Accounts."))?;
        let first = match context.memo.get(SITE_MEMO, context.now).await {
            Some(site) if site.as_str() == Some(Site::China.key()) => Site::China,
            _ => Site::International,
        };
        let mut unreachable = None;
        let mut refused = None;
        for site in [first, first.other()] {
            let response = match http::send(context.http, site.usage_request(key), NAME).await {
                Ok(response) => response,
                Err(error) => {
                    if unreachable.is_none() {
                        unreachable = Some(error);
                    }
                    continue;
                }
            };
            if matches!(response.status, 401 | 403) {
                if refused.is_none() {
                    refused = Some(http::status_error(&response, NAME));
                }
                continue;
            }
            if !response.is_success() {
                return Err(http::status_error(&response, NAME));
            }
            context
                .memo
                .put(
                    SITE_MEMO,
                    json!(site.key()),
                    Some(context.now + Duration::hours(SITE_MEMO_HOURS)),
                )
                .await;
            return read(&http::parse(&response, NAME)?, context.now);
        }
        Err(unreachable
            .or(refused)
            .unwrap_or_else(|| http::decoding(NAME)))
    }
}

/// The reading in an accepted answer: `{code, msg, data: {Response: {Data: {Accounts: [...]}}}}`.
fn read(body: &Value, now: DateTime<Utc>) -> Result<Reading, SimpleProviderError> {
    if let Some(code) = value::number(body, "/code").filter(|code| *code != 0.0) {
        return Err(http::not_available(format!(
            "CodeBuddy could not return usage (error {code})."
        )));
    }
    let response = body
        .pointer("/data/Response")
        .filter(|response| response.is_object())
        .ok_or_else(|| http::decoding(NAME))?;
    if let Some(code) = value::text(response, "/Error/Code") {
        return Err(if code.starts_with("AuthFailure") {
            http::expired("CodeBuddy refused the saved key. Replace it in Accounts.")
        } else {
            http::not_available("CodeBuddy could not return usage. Try again later.")
        });
    }
    let packages: Vec<Package> = response
        .pointer("/Data/Accounts")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| Package::read(entry, now))
        .collect();
    if packages.is_empty() {
        return Err(http::not_available(
            "This CodeBuddy account has no active credit packages.",
        ));
    }
    Ok(Reading::new(plan(&packages), meters(&packages, now)))
}

/// One credit package that still counts at the time of the reading.
struct Package {
    name: Option<String>,
    /// Whether the package refills every cycle instead of expiring after one.
    refills: bool,
    used: f64,
    size: f64,
    left: f64,
    cycle_start: Option<DateTime<Utc>>,
    /// The next refill of a refill package; the end of a one-shot package.
    cycle_end: Option<DateTime<Utc>>,
    /// When the package's credits stop counting.
    expires: Option<DateTime<Utc>>,
}

impl Package {
    /// The package `entry` describes, unless it expired, has not started yet or holds nothing.
    fn read(entry: &Value, now: DateTime<Utc>) -> Option<Self> {
        let time = |field: &str| entry.get(field).and_then(china_time);
        let cycle_start = time("CycleStartTime");
        let cycle_end = time("CycleEndTime");
        let valid_until = time("DeductionEndTime");
        let refills = matches!(
            (cycle_end, valid_until),
            (Some(end), Some(until)) if until - end > Duration::hours(REFILL_GAP_HOURS)
        );
        let expires = if refills {
            valid_until
        } else {
            cycle_end.or(valid_until)
        };
        if expires.is_some_and(|expires| expires <= now)
            || cycle_start.is_some_and(|start| start > now)
        {
            return None;
        }
        let (used, size, left) = if refills {
            figures(entry, "Cycle")
        } else {
            figures(entry, "").or_else(|| figures(entry, "Cycle"))
        }?;
        let name = value::text(entry, "/PackageName")
            .or_else(|| value::text(entry, "/SubProductName"))
            .map(str::to_string);
        Some(Self {
            name,
            refills,
            used,
            size,
            left,
            cycle_start,
            cycle_end,
            expires,
        })
    }

    /// The length of the package's cycle in milliseconds, when both ends are known and plausible.
    fn cycle(&self) -> Option<i64> {
        let length = (self.cycle_end? - self.cycle_start?).num_milliseconds();
        (length > 0 && length <= LONGEST_CYCLE_DAYS * lines::DAY_MS).then_some(length)
    }

    fn daily(&self) -> bool {
        self.cycle()
            .is_some_and(|length| length <= DAILY_CYCLE_HOURS * lines::HOUR_MS)
    }
}

/// A package's used, total and remaining credits from its `<prefix>Capacity*` fields, preferring
/// the exact `*Precise` values; `None` when it holds no credits.
fn figures(entry: &Value, prefix: &str) -> Option<(f64, f64, f64)> {
    let amount = |name: &str| {
        value::number(entry, &format!("/{prefix}Capacity{name}Precise"))
            .or_else(|| value::number(entry, &format!("/{prefix}Capacity{name}")))
    };
    let size = amount("Size").filter(|size| *size > 0.0)?;
    let remain = amount("Remain");
    let used = amount("Used")
        .or_else(|| remain.map(|remain| size - remain))
        .unwrap_or(0.0)
        .max(0.0);
    let left = remain.unwrap_or(size - used).max(0.0);
    Some((used, size, left))
}

/// The Monthly and Daily meters of the refill packages and the Bonus Credits row of the one-shot
/// ones.
fn meters(packages: &[Package], now: DateTime<Utc>) -> Vec<MetricLine> {
    let (refills, one_shot): (Vec<&Package>, Vec<&Package>) =
        packages.iter().partition(|package| package.refills);
    let (daily, monthly): (Vec<&Package>, Vec<&Package>) =
        refills.into_iter().partition(|package| package.daily());
    let mut meters = Vec::new();
    meters.extend(allowance(MONTHLY, &monthly, lines::MONTH_MS, now));
    meters.extend(allowance(DAILY, &daily, lines::DAY_MS, now));
    meters.push(bonus(&one_shot));
    let mut names = std::collections::HashMap::<String, usize>::new();
    for package in packages {
        let base = package.name.as_deref().unwrap_or("Credit Package");
        let base = if [MONTHLY, DAILY, BONUS].contains(&base) {
            format!("Package: {base}")
        } else {
            base.to_string()
        };
        let seen = names.entry(base.clone()).or_default();
        *seen += 1;
        let label = if *seen == 1 {
            base
        } else {
            format!("{base} ({seen})")
        };
        meters.push(if package.refills {
            lines::count(
                &label,
                package.used,
                package.size,
                UNIT,
                package.cycle_end,
                package.cycle(),
            )
        } else {
            MetricLine::Values(ValuesLine {
                label,
                values: vec![MetricValue::count(package.left, UNIT)],
                expiries_at: package.expires.into_iter().collect(),
                ..ValuesLine::default()
            })
        });
    }
    meters
}

/// Refill packages of one cadence as one meter: their credits add up, and the soonest refill resets
/// it over that package's cycle (`period` when the cycle is unknown).
fn allowance(
    label: &str,
    packages: &[&Package],
    period: i64,
    now: DateTime<Utc>,
) -> Option<MetricLine> {
    if packages.is_empty() {
        return None;
    }
    let used: f64 = packages.iter().map(|package| package.used).sum();
    let size: f64 = packages.iter().map(|package| package.size).sum();
    let next = packages
        .iter()
        .filter(|package| package.cycle_end.is_some_and(|end| end > now))
        .min_by_key(|package| package.cycle_end);
    Some(lines::count(
        label,
        used,
        size,
        UNIT,
        next.and_then(|package| package.cycle_end),
        Some(next.and_then(|package| package.cycle()).unwrap_or(period)),
    ))
}

/// One-shot packages as the credits still in them, with when each package that has some left
/// expires.
fn bonus(packages: &[&Package]) -> MetricLine {
    let left: f64 = packages.iter().map(|package| package.left).sum();
    let mut expiries: Vec<DateTime<Utc>> = packages
        .iter()
        .filter(|package| package.left > 0.0)
        .filter_map(|package| package.expires)
        .collect();
    expiries.sort();
    MetricLine::Values(ValuesLine {
        label: BONUS.into(),
        values: vec![MetricValue::count(left, UNIT)],
        expiries_at: expiries,
        ..ValuesLine::default()
    })
}

/// The name of the largest refill package, a monthly one before a daily one.
fn plan(packages: &[Package]) -> Option<String> {
    packages
        .iter()
        .filter(|package| package.refills)
        .max_by(|a, b| {
            (!a.daily())
                .cmp(&!b.daily())
                .then(a.size.total_cmp(&b.size))
        })
        .and_then(|package| package.name.clone())
}

/// A billing time: `2006-01-02 15:04:05` text in China Standard Time (UTC+8), as the service
/// writes it, else RFC 3339 text or an epoch number in seconds or milliseconds.
fn china_time(raw: &Value) -> Option<DateTime<Utc>> {
    if let Some(text) = raw.as_str().map(str::trim) {
        let local = [
            "%Y-%m-%d %H:%M:%S%.f",
            "%Y-%m-%dT%H:%M:%S%.f",
            "%Y/%m/%d %H:%M:%S",
        ]
        .iter()
        .find_map(|format| NaiveDateTime::parse_from_str(text, format).ok())
        .or_else(|| {
            NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .ok()?
                .and_hms_opt(0, 0, 0)
        });
        if let Some(local) = local {
            return FixedOffset::east_opt(8 * 3600)?
                .from_local_datetime(&local)
                .single()
                .map(|time| time.with_timezone(&Utc));
        }
    }
    value::as_time(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{Memo, Roots, Secret};
    use crate::testing::{Scripted, context_at, header};
    use std::sync::Arc;
    use uc_core::{
        ErrorCategory, HttpClient, HttpError, HttpResponse, ProgressFormat, SharedHttpClient,
    };

    const INTERNATIONAL: &str = "https://www.codebuddy.ai/v2/billing/meter/get-user-resource";
    const CHINA: &str = "https://copilot.tencent.com/v2/billing/meter/get-user-resource";

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    /// A wall-clock time in China Standard Time, as the billing service writes them.
    fn china(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> DateTime<Utc> {
        FixedOffset::east_opt(8 * 3600)
            .unwrap()
            .with_ymd_and_hms(year, month, day, hour, minute, second)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn key() -> Value {
        json!({ "apiKey": "ck-test-key" })
    }

    /// A Pro account: two monthly allowances on different cycles, daily promotional credits, a
    /// gift, a top-up, a used-up gift, an expired gift and a pack that starts next month.
    fn packages() -> Value {
        json!([
            {
                "PackageName": "Pro",
                "SubProductName": "CodeBuddy",
                "Status": 0,
                "CycleStartTime": "2026-09-15 00:00:00",
                "CycleEndTime": "2026-10-14 23:59:59",
                "DeductionEndTime": china(2027, 9, 14, 23, 59, 59).timestamp_millis(),
                "CycleCapacitySize": 1000,
                "CycleCapacitySizePrecise": "1000",
                "CycleCapacityUsed": 320,
                "CycleCapacityUsedPrecise": "320.25",
                "CycleCapacityRemain": 679.75,
                "CapacitySize": 12000,
                "CapacityUsed": 3520.25,
                "CapacityRemain": 8479.75
            },
            {
                "PackageName": "Free Base",
                "Status": 0,
                "CycleStartTime": "2026-09-01 00:00:00",
                "CycleEndTime": "2026-09-30 23:59:59",
                "DeductionEndTime": "2049-12-31 23:59:59",
                "CycleCapacitySize": 100,
                "CycleCapacityUsed": 6.54,
                "CycleCapacityRemain": 93.46
            },
            {
                "PackageName": "Daily Bonus",
                "Status": 0,
                "CycleStartTime": "2026-09-27 00:00:00",
                "CycleEndTime": "2026-09-27 23:59:59",
                "DeductionEndTime": china(2026, 12, 31, 23, 59, 59).timestamp_millis(),
                "CycleCapacitySize": 50,
                "CycleCapacityUsedPrecise": "12.5"
            },
            {
                "PackageName": "Activity Gift",
                "Status": 0,
                "CycleStartTime": "2026-09-10 00:00:00",
                "CycleEndTime": "2026-10-10 00:00:00",
                "DeductionEndTime": china(2026, 10, 10, 0, 0, 0).timestamp_millis(),
                "CycleCapacitySize": 100,
                "CycleCapacityUsed": 12,
                "CapacitySize": 100,
                "CapacityUsed": 12,
                "CapacityRemain": 88
            },
            {
                "PackageName": "Top-up Pack",
                "Status": 0,
                "CycleStartTime": "2026-09-20 12:00:00",
                "CycleEndTime": "2027-03-20 12:00:00",
                "DeductionEndTime": "2027-03-20 12:00:00",
                "CapacitySizePrecise": "500",
                "CapacityUsedPrecise": "0"
            },
            {
                "PackageName": "Used Gift",
                "Status": 3,
                "CycleStartTime": "2026-09-21 00:00:00",
                "CycleEndTime": "2026-09-28 00:00:00",
                "DeductionEndTime": "2026-09-28 00:00:00",
                "CapacitySize": 50,
                "CapacityUsed": 50,
                "CapacityRemain": 0
            },
            {
                "PackageName": "Expired Gift",
                "CycleStartTime": "2026-08-26 00:00:00",
                "CycleEndTime": "2026-09-26 00:00:00",
                "CapacitySize": 200,
                "CapacityUsed": 10
            },
            {
                "PackageName": "October Bonus",
                "CycleStartTime": "2026-10-01 00:00:00",
                "CycleEndTime": "2026-10-31 23:59:59",
                "CapacitySize": 300,
                "CapacityUsed": 0
            }
        ])
    }

    fn answer(accounts: Value) -> String {
        let count = accounts.as_array().map_or(0, Vec::len);
        json!({
            "code": 0,
            "msg": "OK",
            "requestId": "req-1",
            "data": {
                "Response": {
                    "Data": { "Accounts": accounts, "TotalCount": count },
                    "RequestId": "req-2"
                }
            }
        })
        .to_string()
    }

    fn progress(line: &MetricLine) -> (&str, f64, f64, Option<DateTime<Utc>>, Option<i64>) {
        let MetricLine::Progress(line) = line else {
            panic!("expected a progress line")
        };
        assert_eq!(
            line.format,
            ProgressFormat::Count {
                suffix: "credits".into()
            }
        );
        (
            line.label.as_str(),
            line.used,
            line.limit,
            line.resets_at,
            line.period_duration_ms,
        )
    }

    #[tokio::test]
    async fn refill_and_one_shot_packages_feed_separate_rows() {
        let http = Scripted::new().on("POST", INTERNATIONAL, 200, &answer(packages()));
        let scope = context_at(&http, key(), now());
        let reading = CodeBuddy.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        let labels: Vec<&str> = reading.lines.iter().map(MetricLine::label).collect();
        assert_eq!(
            labels,
            [
                "Monthly",
                "Daily",
                "Bonus Credits",
                "Pro",
                "Free Base",
                "Daily Bonus",
                "Activity Gift",
                "Top-up Pack",
                "Used Gift"
            ]
        );
        assert_eq!(progress(&reading.lines[3]).1, 320.25);

        let (_, used, limit, resets_at, period) = progress(&reading.lines[0]);
        assert!((used - 326.79).abs() < 1e-9);
        assert_eq!(limit, 1100.0);
        assert_eq!(resets_at, Some(china(2026, 9, 30, 23, 59, 59)));
        assert_eq!(period, Some(30 * lines::DAY_MS - 1000));

        assert_eq!(
            progress(&reading.lines[1]),
            (
                "Daily",
                12.5,
                50.0,
                Some(china(2026, 9, 27, 23, 59, 59)),
                Some(lines::DAY_MS - 1000)
            )
        );

        let MetricLine::Values(bonus) = &reading.lines[2] else {
            panic!("expected a values line")
        };
        assert_eq!(bonus.values, vec![MetricValue::count(588.0, "credits")]);
        assert_eq!(
            bonus.expiries_at,
            vec![china(2026, 10, 10, 0, 0, 0), china(2027, 3, 20, 12, 0, 0)]
        );
    }

    #[tokio::test]
    async fn the_request_carries_the_key_and_the_ide_headers() {
        let http = Scripted::new().on("POST", INTERNATIONAL, 200, &answer(packages()));
        let scope = context_at(&http, key(), now());
        CodeBuddy.fetch(&scope.context()).await.unwrap();
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].url, INTERNATIONAL);
        assert_eq!(
            header(&requests[0], "Authorization"),
            Some("Bearer ck-test-key")
        );
        assert_eq!(
            header(&requests[0], "Content-Type"),
            Some("application/json")
        );
        assert_eq!(
            header(&requests[0], "User-Agent"),
            Some("IDE/2.108.1 CodeBuddy/2.108.1")
        );
        assert_eq!(header(&requests[0], "X-Product"), Some("SaaS"));
        assert_eq!(header(&requests[0], "X-IDE-Type"), Some("IDE"));
        assert_eq!(header(&requests[0], "X-IDE-Name"), Some("IDE"));
        assert_eq!(header(&requests[0], "X-CodeBuddy-Request"), Some("1"));
        let body: Value = serde_json::from_slice(requests[0].body.as_ref().unwrap()).unwrap();
        assert_eq!(body, json!({}));
    }

    #[tokio::test]
    async fn a_key_the_international_site_refuses_is_read_in_china_and_remembered() {
        let http = Scripted::new()
            .on(
                "POST",
                INTERNATIONAL,
                401,
                r#"{"code":401,"msg":"unauthorized"}"#,
            )
            .on("POST", CHINA, 200, &answer(packages()));
        let scope = context_at(&http, key(), now());
        let reading = CodeBuddy.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.lines.len(), 9);
        CodeBuddy.fetch(&scope.context()).await.unwrap();
        let requests = http.requests();
        let urls: Vec<&str> = requests
            .iter()
            .map(|request| request.url.as_str())
            .collect();
        assert_eq!(urls, [INTERNATIONAL, CHINA, CHINA]);
        assert_eq!(
            header(&requests[1], "User-Agent"),
            Some("CLI/2.108.1 CodeBuddy/2.108.1")
        );
        assert_eq!(header(&requests[1], "X-IDE-Type"), Some("CLI"));
        assert_eq!(
            header(&requests[1], "Authorization"),
            Some("Bearer ck-test-key")
        );
    }

    #[tokio::test]
    async fn a_key_both_sites_refuse_is_reported_as_expired() {
        let http = Scripted::new()
            .on("POST", INTERNATIONAL, 401, "{}")
            .on("POST", CHINA, 403, "{}");
        let scope = context_at(&http, key(), now());
        let error = CodeBuddy.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthExpired);
        assert_eq!(
            error.message,
            "CodeBuddy refused the saved login or key. Sign in again or replace the key."
        );
        assert_eq!(http.requests().len(), 2);
    }

    #[tokio::test]
    async fn rate_limiting_is_reported_without_asking_the_other_site() {
        let http = Scripted::new().on("POST", INTERNATIONAL, 429, "{}").on(
            "POST",
            CHINA,
            200,
            &answer(packages()),
        );
        let scope = context_at(&http, key(), now());
        let error = CodeBuddy.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::RateLimited);
        assert_eq!(http.requests().len(), 1);
    }

    /// Answers from `script`, except that requests to `offline` never connect.
    struct PartlyOffline {
        script: Scripted,
        offline: &'static str,
    }

    #[async_trait]
    impl HttpClient for PartlyOffline {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
            if request.url.starts_with(self.offline) {
                return Err(HttpError::Timeout);
            }
            self.script.send(request).await
        }
    }

    #[tokio::test]
    async fn an_unreachable_site_is_skipped_for_the_other_one() {
        let script = Scripted::new().on("POST", CHINA, 200, &answer(packages()));
        let http: SharedHttpClient = Arc::new(PartlyOffline {
            script: script.clone(),
            offline: "https://www.codebuddy.ai/",
        });
        let secret = Secret::new(key());
        let memo = Memo::default();
        let context = FetchContext {
            secret: &secret,
            http: &http,
            now: now(),
            memo: &memo,
        };
        let reading = CodeBuddy.fetch(&context).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        assert_eq!(script.requests().len(), 1);

        let refused = Scripted::new().on("POST", CHINA, 401, "{}");
        let http: SharedHttpClient = Arc::new(PartlyOffline {
            script: refused,
            offline: "https://www.codebuddy.ai/",
        });
        let memo = Memo::default();
        let context = FetchContext {
            secret: &secret,
            http: &http,
            now: now(),
            memo: &memo,
        };
        let error = CodeBuddy.fetch(&context).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Network);
    }

    #[tokio::test]
    async fn an_error_code_in_the_answer_is_reported() {
        let http = Scripted::new().on(
            "POST",
            INTERNATIONAL,
            200,
            r#"{"code":11101,"msg":"invalid request","data":null}"#,
        );
        let scope = context_at(&http, key(), now());
        let error = CodeBuddy.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::NotAvailable);
        assert_eq!(
            error.message,
            "CodeBuddy could not return usage (error 11101)."
        );
    }

    #[tokio::test]
    async fn a_tencent_error_inside_the_answer_is_reported() {
        let failure = |code: &str| {
            json!({
                "code": 0,
                "data": {
                    "Response": {
                        "Error": { "Code": code, "Message": "request failed" },
                        "RequestId": "req-3"
                    }
                }
            })
            .to_string()
        };
        let http = Scripted::new()
            .on(
                "POST",
                INTERNATIONAL,
                200,
                &failure("AuthFailure.TokenFailure"),
            )
            .on("POST", INTERNATIONAL, 200, &failure("InternalError"));
        let scope = context_at(&http, key(), now());
        let error = CodeBuddy.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthExpired);
        assert_eq!(
            error.message,
            "CodeBuddy refused the saved key. Replace it in Accounts."
        );
        let error = CodeBuddy.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::NotAvailable);
        assert_eq!(
            error.message,
            "CodeBuddy could not return usage. Try again later."
        );
    }

    #[tokio::test]
    async fn an_account_without_active_packages_has_no_usage() {
        let expired = json!([packages()[6].clone(), packages()[7].clone()]);
        let http = Scripted::new()
            .on("POST", INTERNATIONAL, 200, &answer(json!([])))
            .on("POST", INTERNATIONAL, 200, &answer(expired));
        let scope = context_at(&http, key(), now());
        for _ in 0..2 {
            let error = CodeBuddy.fetch(&scope.context()).await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::NotAvailable);
            assert_eq!(
                error.message,
                "This CodeBuddy account has no active credit packages."
            );
        }
        assert_eq!(http.requests().len(), 2);
    }

    #[tokio::test]
    async fn an_account_without_one_shot_packages_has_no_bonus_credits_left() {
        let accounts = json!([packages()[1].clone()]);
        let http = Scripted::new().on("POST", INTERNATIONAL, 200, &answer(accounts));
        let scope = context_at(&http, key(), now());
        let reading = CodeBuddy.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Free Base"));
        let labels: Vec<&str> = reading.lines.iter().map(MetricLine::label).collect();
        assert_eq!(labels, ["Monthly", "Bonus Credits", "Free Base"]);
        let MetricLine::Values(bonus) = &reading.lines[1] else {
            panic!("expected a values line")
        };
        assert_eq!(bonus.values, vec![MetricValue::count(0.0, "credits")]);
        assert!(bonus.expiries_at.is_empty());
    }

    #[tokio::test]
    async fn a_missing_key_is_reported_without_a_request() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), now());
        let error = CodeBuddy.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert!(http.requests().is_empty());
    }

    #[test]
    fn billing_times_without_an_offset_are_china_time() {
        let expected = Utc.with_ymd_and_hms(2026, 9, 30, 15, 59, 59).unwrap();
        for raw in [
            json!("2026-09-30 23:59:59"),
            json!("2026-09-30T23:59:59"),
            json!("2026-09-30T15:59:59Z"),
            json!("2026-09-30T23:59:59+08:00"),
            json!(expected.timestamp()),
            json!(expected.timestamp_millis()),
            json!(expected.timestamp_millis().to_string()),
        ] {
            assert_eq!(china_time(&raw), Some(expected), "{raw}");
        }
        assert_eq!(
            china_time(&json!("2026-10-01")),
            Some(Utc.with_ymd_and_hms(2026, 9, 30, 16, 0, 0).unwrap())
        );
        assert_eq!(china_time(&json!("")), None);
        assert_eq!(china_time(&json!(null)), None);
    }

    #[test]
    fn every_row_has_a_widget_and_the_key_is_the_only_connection() {
        let provider = Provider::new("codebuddy@abc", "CodeBuddy");
        let descriptors = CodeBuddy.descriptors(&provider);
        let ids: Vec<&str> = descriptors.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "codebuddy@abc.monthly",
                "codebuddy@abc.daily",
                "codebuddy@abc.bonus"
            ]
        );
        let labels: Vec<&str> = descriptors
            .iter()
            .map(|d| d.metric_label.as_str())
            .collect();
        assert_eq!(labels, ["Monthly", "Daily", "Bonus Credits"]);
        assert_eq!(descriptors[2].limit_resources[0].key, "bonusCredits");

        let connection = CodeBuddy.connection();
        assert!(connection.login_from.is_none());
        let help = connection.api_key.unwrap();
        assert_eq!(help.env, ["CODEBUDDY_API_KEY", "CODEBUDDY_AUTH_TOKEN"]);
        assert_eq!(help.url, "https://www.codebuddy.ai/profile/keys");
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".codebuddy")).unwrap();
        assert!(CodeBuddy.discover(&Roots::under(dir.path())).is_empty());
    }
}
