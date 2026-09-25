//! Date helpers. Port of upstream `OpenUsageISO8601.swift`, `DailyUsageAccumulator.dayKey` and
//! `UsageHistoryWindow`.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use chrono::{DateTime, Days, Local, NaiveDate, SecondsFormat, TimeZone, Utc};
use regex::Regex;

static SPACE_SEPARATED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}").unwrap());
static WITH_ZONE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})(\.\d+)?(Z|[+-]\d{2}:\d{2})$").unwrap()
});
static WITHOUT_ZONE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})(\.\d+)?$").unwrap());

fn normalize(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    if s.is_empty() {
        return s;
    }
    if s.contains(' ')
        && let Some(found) = SPACE_SEPARATED.find(&s)
    {
        let head = found.as_str().replace(' ', "T");
        s = format!("{head}{}", &s[found.end()..]);
    }
    if let Some(stripped) = s.strip_suffix(" UTC") {
        s = format!("{stripped}Z");
    }
    let (captures, assume_utc) = match WITH_ZONE.captures(&s) {
        Some(captures) => (captures, false),
        None => match WITHOUT_ZONE.captures(&s) {
            Some(captures) => (captures, true),
            None => return s,
        },
    };
    let head = captures.get(1).map_or("", |m| m.as_str());
    let fraction = captures.get(2).map(|m| {
        let mut digits: String = m.as_str()[1..].chars().take(3).collect();
        while digits.len() < 3 {
            digits.push('0');
        }
        format!(".{digits}")
    });
    let zone = if assume_utc {
        "Z".to_string()
    } else {
        captures.get(3).map_or("Z", |m| m.as_str()).to_string()
    };
    format!("{head}{}{zone}", fraction.unwrap_or_default())
}

/// Parse the timestamp shapes providers return (space-separated, " UTC" suffix, variable fractional
/// digits, missing zone → UTC).
pub fn parse_iso8601(value: &str) -> Option<DateTime<Utc>> {
    let normalized = normalize(value);
    DateTime::parse_from_rfc3339(&normalized)
        .ok()
        .map(|date| date.with_timezone(&Utc))
}

/// `yyyy-MM-ddTHH:mm:ss.SSSZ`, the upstream wire format.
pub fn format_iso8601(date: &DateTime<Utc>) -> String {
    date.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Parse a Unix timestamp that may be in seconds or milliseconds.
pub fn from_unix_flexible(value: f64) -> Option<DateTime<Utc>> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let millis = if value > 1e11 { value } else { value * 1000.0 };
    Utc.timestamp_millis_opt(millis.round() as i64).single()
}

/// The `yyyy-MM-dd` key of the local calendar day containing `date`.
pub fn day_key<Tz: TimeZone>(date: &DateTime<Tz>) -> String {
    date.with_timezone(&Local).format("%Y-%m-%d").to_string()
}

pub fn day_key_of(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// The shared history window: today plus the previous 30 calendar days.
pub const USAGE_HISTORY_PREVIOUS_DAYS: u64 = 30;

/// Local calendar day keys covered by the history window ending today.
pub fn usage_window_day_keys<Tz: TimeZone>(now: &DateTime<Tz>) -> BTreeSet<String> {
    let today = now.with_timezone(&Local).date_naive();
    (0..=USAGE_HISTORY_PREVIOUS_DAYS)
        .filter_map(|offset| today.checked_sub_days(Days::new(offset)))
        .map(day_key_of)
        .collect()
}

/// Local midnight at the start of the calendar day containing `date`, as UTC.
pub fn start_of_local_day<Tz: TimeZone>(date: &DateTime<Tz>) -> DateTime<Utc> {
    let local_date = date.with_timezone(&Local).date_naive();
    let midnight = local_date.and_hms_opt(0, 0, 0).unwrap_or_default();
    Local
        .from_local_datetime(&midnight)
        .earliest()
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(|| date.with_timezone(&Utc))
}

/// The earliest instant still inside the history window (local midnight 30 days ago).
pub fn usage_window_start<Tz: TimeZone>(now: &DateTime<Tz>) -> DateTime<Utc> {
    let start = start_of_local_day(now);
    start
        .checked_sub_days(Days::new(USAGE_HISTORY_PREVIOUS_DAYS))
        .unwrap_or(start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_provider_shapes() {
        let expected = Utc.with_ymd_and_hms(2026, 9, 25, 14, 30, 0).unwrap();
        assert_eq!(parse_iso8601("2026-09-25T14:30:00Z"), Some(expected));
        assert_eq!(parse_iso8601("2026-09-25 14:30:00 UTC"), Some(expected));
        assert_eq!(parse_iso8601("2026-09-25T14:30:00"), Some(expected));
        assert_eq!(parse_iso8601("2026-09-25T21:30:00+07:00"), Some(expected));
        let fractional = parse_iso8601("2026-09-25T14:30:00.123456789Z").unwrap();
        assert_eq!(fractional.timestamp_subsec_millis(), 123);
        assert_eq!(parse_iso8601("garbage"), None);
    }

    #[test]
    fn formats_with_milliseconds() {
        let date = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
        assert_eq!(format_iso8601(&date), "2026-01-02T03:04:05.000Z");
    }

    #[test]
    fn window_covers_thirty_one_days() {
        let now = Local::now();
        let keys = usage_window_day_keys(&now);
        assert_eq!(keys.len(), 31);
        assert!(keys.contains(&day_key(&now)));
    }

    #[test]
    fn unix_seconds_and_millis_both_parse() {
        let seconds = from_unix_flexible(1_758_800_000.0).unwrap();
        let millis = from_unix_flexible(1_758_800_000_000.0).unwrap();
        assert_eq!(seconds, millis);
    }
}
