//! Windsurf: the daily and weekly quota shares, the message, flow action and flex credit
//! allowances, the plan and the end of its billing period, as the Windsurf app last cached them.
//!
//! The login is the `windsurf.settings.cachedPlanInfo` item of the `ItemTable` in Windsurf's
//! VS Code-style state database, `Windsurf/User/globalStorage/state.vscdb` in the roaming
//! application data folder: `%APPDATA%` on Windows, `~/Library/Application Support` on macOS and
//! `$XDG_CONFIG_HOME` (by default `~/.config`) on Linux. The database is opened read-only where it
//! is, without a copy, and the item's JSON may be stored as text or as UTF-8 or UTF-16LE bytes.
//! The `email` of the app's `windsurfAuthStatus` item, when it holds one, labels the login.
//!
//! A refresh sends no request: it reads the `quotaUsage`, `usage`, `planName` and `endTimestamp`
//! of the JSON the login carries, so the card shows what Windsurf fetched the last time it
//! refreshed its plan.
//!
//! A key pasted in Quota Control (or found in `WINDSURF_API_KEY`) is read live instead. An
//! account's own API key goes to the same `GetUserStatus` Connect RPC on `server.codeium.com` that
//! Devin's card reads (Windsurf and Devin share the account server), for the daily and weekly quota
//! and the extra usage balance. A key that endpoint refuses is tried as an Enterprise team's service
//! key (Billing Read) against `POST https://server.codeium.com/api/v1/GetTeamCreditBalance`, whose
//! add-on credits used and still available this billing cycle make the Add-on Credits meter. The
//! kind of key that answered is remembered for 12 hours.

use async_trait::async_trait;
use chrono::Duration;
use serde_json::{Value, json};
use uc_core::{
    ErrorCategory, HttpRequest, LimitResourceKind, LimitResourceSource, MetricKind, Provider,
    SimpleProviderError, WidgetDescriptor,
};

use super::devin::{StatusCard, USER_STATUS_SERVER, read_user_status};
use crate::service::{
    ApiKeyHelp, Connection, FetchContext, Login, Reading, Roots, Secret, Service,
};
use crate::support::{apps, http, lines, value};

const NAME: &str = "Windsurf";
const EXTRA: &str = "Extra Usage";
const ADD_ON: &str = "Add-on Credits";
const KEY_REFUSED: &str = "Windsurf refused the key. Paste your account's API key, or an Enterprise service key with Billing Read.";
const TEAM_BALANCE: &str = "https://server.codeium.com/api/v1/GetTeamCreditBalance";
/// Memo key: which kind of key answered last, "account" or "team".
const KEY_KIND: &str = "windsurf.key";

/// How a `GetUserStatus` answer is named on Windsurf's card.
const WINDSURF_CARD: StatusCard = StatusCard {
    name: NAME,
    ide: "windsurf",
    expired: KEY_REFUSED,
    unavailable: "Windsurf quota data unavailable. Try again later.",
    daily: "Daily",
    weekly: "Weekly",
    extra: EXTRA,
};

pub(crate) struct Windsurf;

#[async_trait]
impl Service for Windsurf {
    fn id(&self) -> &'static str {
        "windsurf"
    }

    fn name(&self) -> &'static str {
        "Windsurf"
    }

    fn connection(&self) -> Connection {
        Connection::login("Windsurf").or_api_key(ApiKeyHelp {
            env: &["WINDSURF_API_KEY"],
            url: "https://windsurf.com/subscription/usage",
            fields: &[],
        })
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        let path = apps::state_db(roots, "Windsurf");
        let Some((data, email)) = read(&path) else {
            return vec![];
        };
        vec![Login::new("local-windsurf", "Windsurf", &path, Secret::new(data)).with_label(email)]
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        let mut descriptors: Vec<WidgetDescriptor> = [("daily", "Daily"), ("weekly", "Weekly")]
            .into_iter()
            .map(|(id, title)| {
                WidgetDescriptor::percent(
                    format!("{}.{id}", provider.id),
                    provider,
                    title,
                    None,
                    None,
                )
                .exporting_progress(id, "percent")
            })
            .collect();
        descriptors.push(
            WidgetDescriptor::dollar_balance(
                format!("{}.extra", provider.id),
                provider,
                EXTRA,
                None,
                "left",
            )
            .exporting_limit(
                "extraUsageBalance",
                LimitResourceKind::Balance,
                "usd",
                LimitResourceSource::Value {
                    kind: MetricKind::Dollars,
                    label: None,
                },
                false,
            ),
        );
        descriptors.push(
            WidgetDescriptor::percent(
                format!("{}.addOn", provider.id),
                provider,
                ADD_ON,
                None,
                None,
            )
            .exporting_progress("addOnCredits", "percent"),
        );
        descriptors
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        if let Some(key) = context.secret.key() {
            return keyed(context, key).await;
        }
        let plan_info = context.secret.value();
        let mut meters = vec![];
        for (field, reset, title, period) in [
            (
                "dailyRemainingPercent",
                "dailyResetAtUnix",
                "Daily",
                lines::DAY_MS,
            ),
            (
                "weeklyRemainingPercent",
                "weeklyResetAtUnix",
                "Weekly",
                lines::WEEK_MS,
            ),
        ] {
            if let Some(left) = value::number(plan_info, &format!("/quotaUsage/{field}")) {
                meters.push(lines::percent(
                    title,
                    100.0 - left,
                    value::time(plan_info, &format!("/quotaUsage/{reset}")),
                    Some(period),
                ));
            }
        }
        for (total, used, title, unit) in [
            ("messages", "usedMessages", "Messages", "messages"),
            ("flowActions", "usedFlowActions", "Flow Actions", "actions"),
            ("flexCredits", "usedFlexCredits", "Flex Credits", "credits"),
        ] {
            if let (Some(limit), Some(used)) = (
                value::number(plan_info, &format!("/usage/{total}")),
                value::number(plan_info, &format!("/usage/{used}")),
            ) {
                meters.push(lines::count(
                    title,
                    used,
                    limit,
                    unit,
                    value::time(plan_info, "/endTimestamp"),
                    None,
                ));
            }
        }
        if meters.is_empty() {
            return Err(http::not_available(
                "Windsurf has no cached usage. Open Windsurf and refresh its plan.",
            ));
        }
        Ok(Reading::new(
            value::text(plan_info, "/planName").map(str::to_owned),
            meters,
        )
        .with_plan_term(value::time(plan_info, "/endTimestamp").map(|ends_at| {
            uc_core::PlanTerm::Stated {
                ends_at,
                checked_at: None,
            }
        })))
    }
}

/// A pasted key: the account's quota, else the team's add-on credits, starting with the kind of
/// key that answered last.
async fn keyed(context: &FetchContext<'_>, key: &str) -> Result<Reading, SimpleProviderError> {
    let team_first = context
        .memo
        .get(KEY_KIND, context.now)
        .await
        .is_some_and(|kind| kind.as_str() == Some("team"));
    let order: [bool; 2] = if team_first {
        [true, false]
    } else {
        [false, true]
    };
    let mut refused = false;
    let mut failure = None;
    for team in order {
        let result = if team {
            team_balance(context, key).await
        } else {
            read_user_status(context, key, USER_STATUS_SERVER, &WINDSURF_CARD).await
        };
        match result {
            Ok(reading) => {
                let kind = if team { "team" } else { "account" };
                context
                    .memo
                    .put(
                        KEY_KIND,
                        json!(kind),
                        Some(context.now + Duration::hours(12)),
                    )
                    .await;
                return Ok(reading);
            }
            Err(error) if error.category == ErrorCategory::RateLimited => return Err(error),
            Err(error) => {
                refused |= error.category == ErrorCategory::AuthExpired;
                failure.get_or_insert(error);
            }
        }
    }
    if refused {
        return Err(http::invalid(KEY_REFUSED));
    }
    Err(failure.unwrap_or_else(|| http::invalid(KEY_REFUSED)))
}

/// An Enterprise team's add-on credits this billing cycle, read with its service key.
async fn team_balance(
    context: &FetchContext<'_>,
    key: &str,
) -> Result<Reading, SimpleProviderError> {
    let request = HttpRequest::post(TEAM_BALANCE)
        .header("Accept", "application/json")
        .json_body(&json!({ "service_key": key }));
    let response = http::send(context.http, request, NAME).await?;
    match response.status {
        401 | 403 => return Err(http::expired(KEY_REFUSED)),
        _ if !response.is_success() => return Err(http::status_error(&response, NAME)),
        _ => {}
    }
    let body = http::parse(&response, NAME)?;
    let used = value::number(&body, "/addOnCreditsUsed")
        .unwrap_or(0.0)
        .max(0.0);
    let left = value::number(&body, "/addOnCreditsAvailable")
        .ok_or_else(|| http::decoding(NAME))?
        .max(0.0);
    let ends_at = value::time(&body, "/billingCycleEnd");
    let period = value::time(&body, "/billingCycleStart")
        .zip(ends_at)
        .map(|(start, end)| (end - start).num_milliseconds())
        .filter(|period| *period > 0);
    let meter = lines::percent_of(ADD_ON, used, used + left, ends_at, period)
        .unwrap_or_else(|| lines::percent(ADD_ON, 0.0, ends_at, period));
    let seats = value::number(&body, "/numSeats").filter(|seats| *seats > 0.0);
    let plan = match seats {
        Some(seats) => format!("Enterprise · {seats:.0} seats"),
        None => "Enterprise".to_string(),
    };
    Ok(
        Reading::new(Some(plan), vec![meter]).with_plan_term(ends_at.map(|ends_at| {
            uc_core::PlanTerm::Stated {
                ends_at,
                checked_at: None,
            }
        })),
    )
}

/// The `cachedPlanInfo` JSON of the Windsurf state database at `path`, opened read-only in place,
/// and the signed-in account's email from the app's `windsurfAuthStatus` item beside it (the item
/// open-source Windsurf account tools read `email` from), when it names one.
fn read(path: &std::path::Path) -> Option<(Value, Option<String>)> {
    let db = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    db.busy_timeout(std::time::Duration::from_millis(250))
        .ok()?;
    let plan_info = item(&db, "windsurf.settings.cachedPlanInfo")?;
    let email = item(&db, "windsurfAuthStatus")
        .and_then(|status| value::text(&status, "/email").map(str::to_string))
        .filter(|email| email.contains('@'));
    Some((plan_info, email))
}

/// The JSON of one `ItemTable` item. A blob of at most 4 MiB may hold UTF-8 or UTF-16LE text.
fn item(db: &rusqlite::Connection, key: &str) -> Option<Value> {
    let raw: rusqlite::types::Value = db
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1 LIMIT 1",
            [key],
            |row| row.get(0),
        )
        .ok()?;
    match raw {
        rusqlite::types::Value::Text(text) => serde_json::from_str(&text).ok(),
        rusqlite::types::Value::Blob(bytes) => {
            if bytes.len() > 4_194_304 {
                return None;
            }
            if let Ok(parsed) = serde_json::from_slice(&bytes) {
                return Some(parsed);
            }
            if bytes.len() % 2 != 0 {
                return None;
            }
            let chars: Vec<_> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            let text = String::from_utf16(&chars).ok()?;
            serde_json::from_str(text.trim_matches(['\0', '\u{feff}'])).ok()
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at};
    use chrono::Utc;
    use serde_json::json;

    #[tokio::test]
    async fn the_cached_plan_info_gives_the_daily_and_weekly_windows_without_any_request() {
        let http = Scripted::new();
        let scope = context_at(
            &http,
            json!({"planName":"Pro","quotaUsage":{"dailyRemainingPercent":75,"weeklyRemainingPercent":90,"dailyResetAtUnix":1790503200,"weeklyResetAtUnix":1790812800}}),
            Utc::now(),
        );
        let reading = Windsurf.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        assert_eq!(
            reading.lines,
            vec![
                lines::percent(
                    "Daily",
                    25.0,
                    value::as_time(&json!(1790503200)),
                    Some(lines::DAY_MS)
                ),
                lines::percent(
                    "Weekly",
                    10.0,
                    value::as_time(&json!(1790812800)),
                    Some(lines::WEEK_MS)
                )
            ]
        );
        assert!(http.requests().is_empty());
    }

    const STATUS: &str =
        "https://server.codeium.com/exa.seat_management_pb.SeatManagementService/GetUserStatus";

    #[tokio::test]
    async fn an_account_key_reads_the_live_quota_under_windsurfs_titles() {
        let answer = json!({"userStatus": {"planStatus": {
            "planInfo": {"planName": "Pro"},
            "dailyQuotaRemainingPercent": 80,
            "weeklyQuotaRemainingPercent": 60,
            "overageBalanceMicros": "2500000"
        }}});
        let http = Scripted::new().on("POST", STATUS, 200, &answer.to_string());
        let scope = context_at(&http, json!({"apiKey": "ws-key"}), Utc::now());
        let reading = Windsurf.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        let labels: Vec<&str> = reading.lines.iter().map(|line| line.label()).collect();
        assert_eq!(labels, vec!["Daily", "Weekly", EXTRA]);
        let request = &http.requests()[0];
        let body: Value = serde_json::from_slice(request.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["metadata"]["apiKey"], "ws-key");
        assert_eq!(body["metadata"]["ideName"], "windsurf");
    }

    #[tokio::test]
    async fn a_service_key_the_account_endpoint_refuses_reads_the_teams_add_on_credits() {
        let http = Scripted::new().on("POST", STATUS, 401, "{}").on(
            "POST",
            TEAM_BALANCE,
            200,
            r#"{"promptCreditsPerSeat":500,"numSeats":50,"addOnCreditsAvailable":6500,
                    "addOnCreditsUsed":3500,"billingCycleStart":"2026-09-01T00:00:00Z",
                    "billingCycleEnd":"2026-10-01T00:00:00Z"}"#,
        );
        let scope = context_at(&http, json!({"apiKey": "svc-key"}), Utc::now());
        let reading = Windsurf.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Enterprise · 50 seats"));
        assert_eq!(reading.lines.len(), 1);
        assert_eq!(reading.lines[0].label(), ADD_ON);
        let requests = http.requests();
        let body: Value = serde_json::from_slice(requests[1].body.as_deref().unwrap()).unwrap();
        assert_eq!(body, json!({"service_key": "svc-key"}));
        let again = Windsurf.fetch(&scope.context()).await.unwrap();
        assert_eq!(again.lines.len(), 1);
        assert_eq!(
            http.requests().len(),
            3,
            "the team route is tried first once it answered"
        );
    }

    #[tokio::test]
    async fn a_key_both_endpoints_refuse_says_which_keys_work() {
        let http =
            Scripted::new()
                .on("POST", STATUS, 401, "{}")
                .on("POST", TEAM_BALANCE, 403, "{}");
        let scope = context_at(&http, json!({"apiKey": "bad"}), Utc::now());
        let error = Windsurf.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.message, KEY_REFUSED);
    }

    #[test]
    fn discovers_a_plan_info_saved_as_utf_16_and_nothing_without_a_database() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path());
        assert!(Windsurf.discover(&roots).is_empty());
        let path = apps::state_db(&roots, "Windsurf");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE ItemTable(key TEXT,value BLOB)")
            .unwrap();
        let bytes: Vec<u8> = r#"{"planName":"Pro","quotaUsage":{"dailyRemainingPercent":50}}"#
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        db.execute(
            "INSERT INTO ItemTable VALUES('windsurf.settings.cachedPlanInfo',?1)",
            [bytes],
        )
        .unwrap();
        drop(db);
        let logins = Windsurf.discover(&roots);
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].secret.str("/planName"), Some("Pro"));
        assert_eq!(logins[0].label, None);
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute(
            "INSERT INTO ItemTable VALUES('windsurfAuthStatus',?1)",
            [r#"{"name":"Minh","apiKey":"sk-ws","email":"minh@example.com"}"#],
        )
        .unwrap();
        drop(db);
        let logins = Windsurf.discover(&roots);
        assert_eq!(logins[0].label.as_deref(), Some("minh@example.com"));
    }
}
