//! Provider output normalized into a small app-owned vocabulary.
//!
//! Port of upstream `MetricLine.swift`, `MetricValue.swift` and `MetricKind.swift`. The serde
//! representation matches the upstream `Codable` JSON exactly (same keys, same tagging), so cached
//! snapshots and the local HTTP API stay wire-compatible with OpenUsage tooling.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::usage::ModelUsageBreakdown;

/// How a metric's number is formatted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MetricKind {
    /// `used` is 0...100.
    Percent,
    /// `used` is an amount in USD.
    Dollars,
    /// `used` is an absolute count (with an optional suffix).
    Count,
}

/// Formatting of a bounded `progress` line.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ProgressFormat {
    Percent,
    Dollars,
    Count { suffix: String },
}

impl ProgressFormat {
    pub fn metric_kind(&self) -> MetricKind {
        match self {
            ProgressFormat::Percent => MetricKind::Percent,
            ProgressFormat::Dollars => MetricKind::Dollars,
            ProgressFormat::Count { .. } => MetricKind::Count,
        }
    }

    pub fn count_suffix(&self) -> Option<&str> {
        match self {
            ProgressFormat::Count { suffix } => Some(suffix),
            _ => None,
        }
    }
}

/// One measured number on a metric row, carried raw so formatting happens only at the display edge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricValue {
    /// USD for `dollars`, 0...100 for `percent`, an absolute count otherwise.
    pub number: f64,
    pub kind: MetricKind,
    /// Unit noun shown after the number ("tokens", "credits", "available").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// True when the number is imputed locally rather than measured or billed.
    #[serde(default)]
    pub estimated: bool,
}

impl MetricValue {
    pub fn new(number: f64, kind: MetricKind) -> Self {
        Self { number, kind, label: None, estimated: false }
    }

    pub fn dollars(number: f64) -> Self {
        Self::new(number, MetricKind::Dollars)
    }

    pub fn count(number: f64, label: impl Into<String>) -> Self {
        Self { number, kind: MetricKind::Count, label: Some(label.into()), estimated: false }
    }

    pub fn estimated(mut self, estimated: bool) -> Self {
        self.estimated = estimated;
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

/// One column of a `chart` line: a day's value, its axis label ("Jun 21") and the pre-formatted
/// readout shown on hover ("222M tokens").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricChartPoint {
    pub value: f64,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_label: Option<String>,
}

/// A string-valued provider notice preserved for the local API.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextLine {
    pub label: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
}

/// A small day-by-day bar chart (the Usage Trend row).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartLine {
    pub label: String,
    pub points: Vec<MetricChartPoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// An unbounded row carrying one or more raw numbers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValuesLine {
    pub label: String,
    pub values: Vec<MetricValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_hex: Option<String>,
    /// Future expiry instants surfaced in the row's hover tooltip (Codex reset credits).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expiries_at: Vec<DateTime<Utc>>,
    /// Models this period's spend used that no pricing source can price.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unknown_models: Vec<String>,
    /// Period-scoped ranked model list for spend rows (UI data, not part of the limits contract).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_breakdown: Option<ModelUsageBreakdown>,
}

/// A bounded meter with a `used`/`limit` pair.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressLine {
    pub label: String,
    pub used: f64,
    pub limit: f64,
    pub format: ProgressFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period_duration_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_hex: Option<String>,
}

/// A short status pill (e.g. "Disabled", or a pay-as-you-go cap).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BadgeLine {
    pub label: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
}

/// One normalized metric row produced by a provider refresh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum MetricLine {
    Text(TextLine),
    Chart(ChartLine),
    Values(ValuesLine),
    Progress(ProgressLine),
    Badge(BadgeLine),
}

impl MetricLine {
    /// The badge label that marks a provider-level error line.
    pub const ERROR_BADGE_LABEL: &'static str = "Error";

    pub fn label(&self) -> &str {
        match self {
            MetricLine::Text(line) => &line.label,
            MetricLine::Chart(line) => &line.label,
            MetricLine::Values(line) => &line.label,
            MetricLine::Progress(line) => &line.label,
            MetricLine::Badge(line) => &line.label,
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, MetricLine::Badge(badge) if badge.label == Self::ERROR_BADGE_LABEL)
    }

    /// The shared "no usage data" placeholder badge, shown when a provider returns no metric lines.
    pub fn no_usage_data() -> Self {
        MetricLine::Badge(BadgeLine {
            label: "Status".into(),
            text: "No usage data".into(),
            color_hex: Some("#A3A3A3".into()),
            subtitle: None,
        })
    }

    /// Append `no_usage_data` when nothing was produced.
    pub fn append_no_data_if_needed(lines: &mut Vec<MetricLine>) {
        if lines.is_empty() {
            lines.push(Self::no_usage_data());
        }
    }

    pub fn progress(label: impl Into<String>, used: f64, limit: f64, format: ProgressFormat) -> ProgressLine {
        ProgressLine {
            label: label.into(),
            used,
            limit,
            format,
            resets_at: None,
            period_duration_ms: None,
            color_hex: None,
        }
    }

    pub fn values(label: impl Into<String>, values: Vec<MetricValue>) -> ValuesLine {
        ValuesLine { label: label.into(), values, ..ValuesLine::default() }
    }

    pub fn badge(label: impl Into<String>, text: impl Into<String>) -> BadgeLine {
        BadgeLine { label: label.into(), text: text.into(), ..BadgeLine::default() }
    }

    pub fn text(label: impl Into<String>, value: impl Into<String>) -> TextLine {
        TextLine { label: label.into(), value: value.into(), ..TextLine::default() }
    }
}

impl ProgressLine {
    pub fn resets_at(mut self, resets_at: Option<DateTime<Utc>>) -> Self {
        self.resets_at = resets_at;
        self
    }

    pub fn period_ms(mut self, period_duration_ms: Option<i64>) -> Self {
        self.period_duration_ms = period_duration_ms;
        self
    }

    pub fn color(mut self, color_hex: impl Into<String>) -> Self {
        self.color_hex = Some(color_hex.into());
        self
    }
}

impl BadgeLine {
    pub fn color(mut self, color_hex: impl Into<String>) -> Self {
        self.color_hex = Some(color_hex.into());
        self
    }

    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }
}

impl From<TextLine> for MetricLine {
    fn from(line: TextLine) -> Self {
        MetricLine::Text(line)
    }
}

impl From<ChartLine> for MetricLine {
    fn from(line: ChartLine) -> Self {
        MetricLine::Chart(line)
    }
}

impl From<ValuesLine> for MetricLine {
    fn from(line: ValuesLine) -> Self {
        MetricLine::Values(line)
    }
}

impl From<ProgressLine> for MetricLine {
    fn from(line: ProgressLine) -> Self {
        MetricLine::Progress(line)
    }
}

impl From<BadgeLine> for MetricLine {
    fn from(line: BadgeLine) -> Self {
        MetricLine::Badge(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_line_round_trips_with_upstream_keys() {
        let line: MetricLine = MetricLine::progress("Session", 12.0, 100.0, ProgressFormat::Percent)
            .period_ms(Some(18_000_000))
            .into();
        let json = serde_json::to_value(&line).unwrap();
        assert_eq!(json["type"], "progress");
        assert_eq!(json["format"]["kind"], "percent");
        assert_eq!(json["periodDurationMs"], 18_000_000);
        assert!(json.get("resetsAt").is_none());
        let back: MetricLine = serde_json::from_value(json).unwrap();
        assert_eq!(back, line);
    }

    #[test]
    fn count_format_carries_suffix() {
        let format = ProgressFormat::Count { suffix: "requests".into() };
        let json = serde_json::to_value(&format).unwrap();
        assert_eq!(json, serde_json::json!({"kind": "count", "suffix": "requests"}));
    }

    #[test]
    fn values_line_omits_empty_collections() {
        let line: MetricLine = MetricLine::values("Today", vec![MetricValue::dollars(4.08).estimated(true)]).into();
        let json = serde_json::to_value(&line).unwrap();
        assert!(json.get("expiriesAt").is_none());
        assert!(json.get("unknownModels").is_none());
        assert_eq!(json["values"][0]["estimated"], true);
    }

    #[test]
    fn error_badge_is_detected() {
        let line: MetricLine = MetricLine::badge(MetricLine::ERROR_BADGE_LABEL, "Not logged in").into();
        assert!(line.is_error());
        assert!(!MetricLine::no_usage_data().is_error());
    }
}
