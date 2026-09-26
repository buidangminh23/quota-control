use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serializer;

/// Upstream `OpenUsageISO8601`: UTC with milliseconds, e.g. `2026-07-13T01:40:00.000Z`.
pub fn iso(date: DateTime<Utc>) -> String {
    date.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Whole numbers as JSON integers (`42`, not `42.0`), like upstream's `JSONEncoder`.
pub fn number<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        serializer.serialize_i64(*value as i64)
    } else {
        serializer.serialize_f64(*value)
    }
}

pub fn optional_number<S: Serializer>(
    value: &Option<f64>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(value) => number(value, serializer),
        None => serializer.serialize_none(),
    }
}
