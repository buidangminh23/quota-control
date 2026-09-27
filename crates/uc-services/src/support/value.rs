//! Reading loosely typed JSON: numbers that arrive as strings, times as ISO text or epoch numbers.

use std::path::Path;

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

/// A JSON file of at most `max_bytes`, or `None` when it is missing, too large or not JSON.
pub fn read_json(path: &Path, max_bytes: u64) -> Option<Value> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

/// The number at `pointer`, from a JSON number or a numeric string.
pub fn number(value: &Value, pointer: &str) -> Option<f64> {
    as_number(value.pointer(pointer)?)
}

pub fn as_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|number| number.is_finite())
}

/// The non-empty trimmed string at `pointer`.
pub fn text<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// The boolean at `pointer`, accepting `"true"`/`"false"` strings.
pub fn flag(value: &Value, pointer: &str) -> Option<bool> {
    match value.pointer(pointer)? {
        Value::Bool(flag) => Some(*flag),
        Value::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// The time at `pointer`: RFC 3339 text, a date-time without offset (read as UTC), a date, or an
/// epoch number in seconds or milliseconds (numbers above 10^12 are milliseconds).
pub fn time(value: &Value, pointer: &str) -> Option<DateTime<Utc>> {
    as_time(value.pointer(pointer)?)
}

pub fn as_time(value: &Value) -> Option<DateTime<Utc>> {
    match value {
        Value::Number(_) => epoch(as_number(value)?),
        Value::String(text) => {
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            if let Ok(number) = text.parse::<f64>() {
                return epoch(number);
            }
            parse_time_text(text)
        }
        _ => None,
    }
}

fn epoch(number: f64) -> Option<DateTime<Utc>> {
    if number <= 0.0 {
        return None;
    }
    let millis = if number >= 1e12 {
        number
    } else {
        number * 1000.0
    };
    Utc.timestamp_millis_opt(millis.round() as i64).single()
}

fn parse_time_text(text: &str) -> Option<DateTime<Utc>> {
    if let Ok(time) = DateTime::parse_from_rfc3339(text) {
        return Some(time.with_timezone(&Utc));
    }
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(time) = chrono::NaiveDateTime::parse_from_str(text, format) {
            return Some(Utc.from_utc_datetime(&time));
        }
    }
    for format in ["%Y-%m-%d %H:%M:%S %z", "%a, %d %b %Y %H:%M:%S %z"] {
        if let Ok(time) = DateTime::parse_from_str(text, format) {
            return Some(time.with_timezone(&Utc));
        }
    }
    let date = chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()?;
    Some(Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numbers_come_from_numbers_or_numeric_strings() {
        let value = json!({"a": 3, "b": "4.5", "c": "x", "d": null});
        assert_eq!(number(&value, "/a"), Some(3.0));
        assert_eq!(number(&value, "/b"), Some(4.5));
        assert_eq!(number(&value, "/c"), None);
        assert_eq!(number(&value, "/d"), None);
    }

    #[test]
    fn times_accept_text_and_epoch_forms() {
        let expected = Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap();
        for value in [
            json!("2026-09-27T10:00:00Z"),
            json!("2026-09-27T17:00:00+07:00"),
            json!("2026-09-27T10:00:00"),
            json!("2026-09-27 10:00:00"),
            json!(1790503200),
            json!(1790503200000_i64),
            json!("1790503200"),
        ] {
            assert_eq!(as_time(&value), Some(expected), "{value}");
        }
        assert_eq!(
            as_time(&json!("2026-09-27")),
            Some(Utc.with_ymd_and_hms(2026, 9, 27, 0, 0, 0).unwrap())
        );
        assert_eq!(as_time(&json!(0)), None);
        assert_eq!(as_time(&json!("soon")), None);
    }

    #[test]
    fn json_files_may_start_with_a_byte_order_mark() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.json");
        std::fs::write(&path, "\u{feff}{\"x\":1}").unwrap();
        assert_eq!(read_json(&path, 100), Some(json!({"x": 1})));
        assert_eq!(read_json(&path, 2), None);
        assert_eq!(read_json(&dir.path().join("missing.json"), 100), None);
    }
}
