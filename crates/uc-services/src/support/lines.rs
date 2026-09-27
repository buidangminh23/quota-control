//! Building the metric lines a service's widgets read. A line's `label` must equal the metric label
//! of the widget that shows it (the widget title unless the descriptor names another label).

use chrono::{DateTime, Utc};
use uc_core::{
    BadgeLine, MetricKind, MetricLine, MetricValue, ProgressFormat, ProgressLine, TextLine,
    ValuesLine,
};

pub const HOUR_MS: i64 = 3_600_000;
pub const DAY_MS: i64 = 24 * HOUR_MS;
pub const WEEK_MS: i64 = 7 * DAY_MS;
/// A 30-day month, the length meters use for a monthly window's pace.
pub const MONTH_MS: i64 = 30 * DAY_MS;

/// A window's used share as a percent meter (`used_percent` is clamped to 0...100).
pub fn percent(
    label: &str,
    used_percent: f64,
    resets_at: Option<DateTime<Utc>>,
    period_ms: Option<i64>,
) -> MetricLine {
    MetricLine::Progress(ProgressLine {
        label: label.into(),
        used: clamp_percent(used_percent),
        limit: 100.0,
        format: ProgressFormat::Percent,
        resets_at,
        period_duration_ms: period_ms,
        color_hex: None,
    })
}

/// `used` of `limit` things as a percent meter, when only the share matters.
pub fn percent_of(
    label: &str,
    used: f64,
    limit: f64,
    resets_at: Option<DateTime<Utc>>,
    period_ms: Option<i64>,
) -> Option<MetricLine> {
    (limit > 0.0 && used.is_finite())
        .then(|| percent(label, used / limit * 100.0, resets_at, period_ms))
}

/// `used` of `limit` counted things ("120 / 300 requests").
pub fn count(
    label: &str,
    used: f64,
    limit: f64,
    suffix: &str,
    resets_at: Option<DateTime<Utc>>,
    period_ms: Option<i64>,
) -> MetricLine {
    MetricLine::Progress(ProgressLine {
        label: label.into(),
        used: used.max(0.0),
        limit: limit.max(0.0),
        format: ProgressFormat::Count {
            suffix: suffix.into(),
        },
        resets_at,
        period_duration_ms: period_ms,
        color_hex: None,
    })
}

/// `used` of `limit` US dollars.
pub fn dollars(
    label: &str,
    used: f64,
    limit: f64,
    resets_at: Option<DateTime<Utc>>,
    period_ms: Option<i64>,
) -> MetricLine {
    MetricLine::Progress(ProgressLine {
        label: label.into(),
        used: used.max(0.0),
        limit: limit.max(0.0),
        format: ProgressFormat::Dollars,
        resets_at,
        period_duration_ms: period_ms,
        color_hex: None,
    })
}

/// An unbounded row of one or more numbers ("$12.40 left", "3 credits").
pub fn values(label: &str, values: Vec<MetricValue>) -> MetricLine {
    MetricLine::Values(ValuesLine {
        label: label.into(),
        values,
        ..Default::default()
    })
}

/// A dollar amount (a balance or a spend) as an unbounded row.
pub fn dollar_value(label: &str, amount: f64) -> MetricLine {
    values(label, vec![MetricValue::dollars(amount)])
}

/// A count with its unit word ("1,200 credits").
pub fn count_value(label: &str, amount: f64, unit: &str) -> MetricLine {
    values(label, vec![MetricValue::count(amount, unit)])
}

/// A short status pill.
pub fn badge(label: &str, text: &str) -> MetricLine {
    MetricLine::Badge(BadgeLine {
        label: label.into(),
        text: text.into(),
        color_hex: None,
        subtitle: None,
    })
}

/// A text notice kept for the local API.
pub fn text(label: &str, value: &str) -> MetricLine {
    MetricLine::Text(TextLine {
        label: label.into(),
        value: value.into(),
        color_hex: None,
        subtitle: None,
    })
}

pub fn clamp_percent(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 100.0)
    } else {
        0.0
    }
}

/// A plan or tier id turned into a display name: `pro_plus` → `Pro Plus`, `FREE` → `Free`.
pub fn plan_name(raw: &str) -> Option<String> {
    let words: Vec<String> = raw
        .split(|character: char| character == '_' || character == '-' || character.is_whitespace())
        .filter(|word| !word.is_empty())
        .map(|word| {
            let lower = word.to_lowercase();
            let mut characters = lower.chars();
            match characters.next() {
                Some(first) => first.to_uppercase().chain(characters).collect(),
                None => String::new(),
            }
        })
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

/// The unit a `MetricKind` is shown in; handy when a service reports mixed kinds.
pub fn value(number: f64, kind: MetricKind, label: Option<&str>) -> MetricValue {
    let value = MetricValue::new(number, kind);
    match label {
        Some(label) => value.with_label(label),
        None => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_meters_clamp_and_ratios_skip_empty_limits() {
        let MetricLine::Progress(line) = percent("Session", 130.0, None, None) else {
            panic!("progress")
        };
        assert_eq!(line.used, 100.0);
        assert!(percent_of("Session", 1.0, 0.0, None, None).is_none());
        let Some(MetricLine::Progress(line)) = percent_of("Session", 25.0, 200.0, None, None)
        else {
            panic!("progress")
        };
        assert_eq!(line.used, 12.5);
    }

    #[test]
    fn plan_names_are_title_cased() {
        assert_eq!(plan_name("pro_plus").as_deref(), Some("Pro Plus"));
        assert_eq!(plan_name("FREE").as_deref(), Some("Free"));
        assert_eq!(plan_name("  ").as_deref(), None);
    }
}
