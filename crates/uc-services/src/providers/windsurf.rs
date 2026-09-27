//! Windsurf: the daily and weekly quota shares, the message, flow action and flex credit
//! allowances, the plan and the end of its billing period, as the Windsurf app last cached them.
//!
//! The login is the `windsurf.settings.cachedPlanInfo` item of the `ItemTable` in Windsurf's
//! VS Code-style state database, `Windsurf/User/globalStorage/state.vscdb` in the roaming
//! application data folder: `%APPDATA%` on Windows, `~/Library/Application Support` on macOS and
//! `$XDG_CONFIG_HOME` (by default `~/.config`) on Linux. The database is opened read-only where it
//! is, without a copy, and the item's JSON may be stored as text or as UTF-8 or UTF-16LE bytes.
//!
//! A refresh sends no request: it reads the `quotaUsage`, `usage`, `planName` and `endTimestamp`
//! of the JSON the login carries, so the card shows what Windsurf fetched the last time it
//! refreshed its plan.

use async_trait::async_trait;
use serde_json::Value;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Login, Reading, Roots, Secret, Service};
use crate::support::{apps, http, lines, value};

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
        Connection::login("Windsurf")
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        let path = apps::state_db(roots, "Windsurf");
        let Some(data) = read(&path) else {
            return vec![];
        };
        vec![Login::new(
            "local-windsurf",
            "Windsurf",
            &path,
            Secret::new(data),
        )]
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        [("daily", "Daily"), ("weekly", "Weekly")]
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
            .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
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

/// The `cachedPlanInfo` JSON of the Windsurf state database at `path`, opened read-only in place.
/// A blob of at most 4 MiB may hold UTF-8 or UTF-16LE text.
fn read(path: &std::path::Path) -> Option<Value> {
    let db = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    db.busy_timeout(std::time::Duration::from_millis(250))
        .ok()?;
    let raw: rusqlite::types::Value = db
        .query_row(
            "SELECT value FROM ItemTable WHERE key = 'windsurf.settings.cachedPlanInfo' LIMIT 1",
            [],
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
    }
}
