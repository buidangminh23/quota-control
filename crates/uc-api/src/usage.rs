//! The legacy `/v1/usage` shape (upstream `LocalUsageAPI.WireSnapshot`): UI-oriented rows kept for
//! existing integrations. New consumers should read `/v1/limits`.

use serde::Serialize;
use uc_core::format::{Style, value_string};
use uc_core::{
    MetricChartPoint, MetricKind, MetricLine, MetricValue, ProgressFormat, ProviderSnapshot,
};

use crate::wire::{iso, number};

pub fn encode(snapshots: &[&ProviderSnapshot]) -> Vec<u8> {
    let wire: Vec<WireSnapshot> = snapshots
        .iter()
        .map(|snapshot| WireSnapshot::new(snapshot))
        .collect();
    serde_json::to_vec(&wire).unwrap_or_else(|_| b"[]".to_vec())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireSnapshot<'a> {
    provider_id: &'a str,
    display_name: &'a str,
    plan: Option<&'a str>,
    lines: Vec<WireLine<'a>>,
    fetched_at: String,
}

impl<'a> WireSnapshot<'a> {
    fn new(snapshot: &'a ProviderSnapshot) -> Self {
        Self {
            provider_id: &snapshot.provider_id,
            display_name: &snapshot.display_name,
            plan: snapshot.plan.as_deref(),
            lines: snapshot
                .lines
                .iter()
                .filter(|line| !line.is_error())
                .map(WireLine::new)
                .collect(),
            fetched_at: iso(snapshot.refreshed_at),
        }
    }
}

#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum WireLine<'a> {
    Text {
        label: &'a str,
        value: String,
        color: Option<&'a str>,
        subtitle: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        resets_at: Option<String>,
    },
    Progress {
        label: &'a str,
        #[serde(serialize_with = "number")]
        used: f64,
        #[serde(serialize_with = "number")]
        limit: f64,
        format: &'a ProgressFormat,
        #[serde(skip_serializing_if = "Option::is_none")]
        resets_at: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        period_duration_ms: Option<i64>,
        color: Option<&'a str>,
    },
    Badge {
        label: &'a str,
        text: &'a str,
        color: Option<&'a str>,
        subtitle: Option<&'a str>,
    },
    BarChart {
        label: &'a str,
        points: &'a [MetricChartPoint],
        #[serde(skip_serializing_if = "Option::is_none")]
        note: Option<&'a str>,
        color: Option<&'a str>,
    },
}

impl<'a> WireLine<'a> {
    fn new(line: &'a MetricLine) -> Self {
        match line {
            MetricLine::Text(text) => Self::Text {
                label: &text.label,
                value: text.value.clone(),
                color: text.color_hex.as_deref(),
                subtitle: text.subtitle.as_deref(),
                resets_at: None,
            },
            MetricLine::Values(values) => Self::Text {
                label: &values.label,
                value: legacy_value(&values.values),
                color: values.color_hex.as_deref(),
                subtitle: None,
                resets_at: values.expiries_at.iter().min().copied().map(iso),
            },
            MetricLine::Progress(progress) => Self::Progress {
                label: &progress.label,
                used: progress.used,
                limit: progress.limit,
                format: &progress.format,
                resets_at: progress.resets_at.map(iso),
                period_duration_ms: progress.period_duration_ms,
                color: progress.color_hex.as_deref(),
            },
            MetricLine::Badge(badge) => Self::Badge {
                label: &badge.label,
                text: &badge.text,
                color: badge.color_hex.as_deref(),
                subtitle: badge.subtitle.as_deref(),
            },
            MetricLine::Chart(chart) => Self::BarChart {
                label: &chart.label,
                points: &chart.points,
                note: chart.note.as_deref(),
                color: None,
            },
        }
    }
}

/// The combined string a values row used to carry ("$5.17 · 9.2M tokens"): dollars in full so
/// cents survive, counts compact.
fn legacy_value(values: &[MetricValue]) -> String {
    values
        .iter()
        .map(|value| {
            let style = if value.kind == MetricKind::Count {
                Style::Tray
            } else {
                Style::Full
            };
            value_string(value, style)
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use serde_json::{Value, json};
    use uc_core::{ChartLine, Provider};

    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn encoded(snapshot: &ProviderSnapshot) -> Value {
        serde_json::from_slice(&encode(&[snapshot])).unwrap()
    }

    #[test]
    fn rows_keep_the_documented_legacy_shape() {
        let provider = Provider::new("claude", "Claude");
        let mut today = MetricLine::values(
            "Today",
            vec![
                MetricValue::dollars(5.17),
                MetricValue::count(9_200_000.0, "tokens"),
            ],
        );
        today.expiries_at = vec![at("2026-04-01T00:00:00Z"), at("2026-03-28T00:00:00Z")];
        let snapshot = ProviderSnapshot::make(
            &provider,
            Some("Team 5x".into()),
            vec![
                MetricLine::progress("Session", 42.0, 100.0, ProgressFormat::Percent)
                    .resets_at(Some(at("2026-03-26T13:00:00.161Z")))
                    .period_ms(Some(18_000_000))
                    .into(),
                today.into(),
                MetricLine::badge("Pay as you go", "2500 cap")
                    .color("#22c55e")
                    .into(),
                ChartLine {
                    label: "Usage Trend".into(),
                    points: vec![MetricChartPoint {
                        value: 1_200_000.0,
                        label: "Mar 25".into(),
                        value_label: Some("1.2M tokens".into()),
                    }],
                    note: Some("Estimated from local Claude logs at API rates.".into()),
                }
                .into(),
            ],
            at("2026-03-26T11:16:29Z"),
        );
        let root = encoded(&snapshot);
        let entry = &root[0];
        assert_eq!(entry["providerId"], "claude");
        assert_eq!(entry["plan"], "Team 5x");
        assert_eq!(entry["fetchedAt"], "2026-03-26T11:16:29.000Z");
        let lines = entry["lines"].as_array().unwrap();
        assert_eq!(
            lines[0],
            json!({"type":"progress","label":"Session","used":42,"limit":100,"format":{"kind":"percent"},"resetsAt":"2026-03-26T13:00:00.161Z","periodDurationMs":18000000,"color":null})
        );
        assert_eq!(
            lines[1],
            json!({"type":"text","label":"Today","value":"$5.17 · 9.2M tokens","color":null,"subtitle":null,"resetsAt":"2026-03-28T00:00:00.000Z"})
        );
        assert_eq!(
            lines[2],
            json!({"type":"badge","label":"Pay as you go","text":"2500 cap","color":"#22c55e","subtitle":null})
        );
        assert_eq!(lines[3]["type"], "barChart");
        assert_eq!(lines[3]["points"][0]["valueLabel"], "1.2M tokens");
        assert_eq!(lines[3]["color"], Value::Null);
    }

    #[test]
    fn a_missing_plan_is_an_explicit_null() {
        let snapshot = ProviderSnapshot::make(
            &Provider::new("codex", "Codex"),
            None,
            vec![],
            at("2026-03-26T11:16:29Z"),
        );
        let root = encoded(&snapshot);
        assert!(root[0].as_object().unwrap().contains_key("plan"));
        assert_eq!(root[0]["plan"], Value::Null);
    }
}
