//! `/v1/limits`, the machine-facing contract shared by the CLI and the local HTTP API (upstream
//! `LocalLimitsAPI.swift`). It selects only the scalars descriptors explicitly export and names
//! them with stable keys; colors, subtitles, charts and spend history never reach it.

use std::collections::BTreeMap;

use chrono::Duration;
use serde::Serialize;
use uc_core::{
    LimitResourceDescriptor, LimitResourceKind, LimitResourceSource, MetricLine, ProgressFormat,
    ProgressLine, ProviderSnapshot, WidgetDescriptor,
};

use crate::state::ApiState;
use crate::wire::{iso, optional_number};

/// Kept from upstream so tools that check it accept Quota Control's output.
pub const SCHEMA: &str = "openusage.limits.v1";

/// The envelope for `provider_ids`: every one with a snapshot and exported limits, plus the
/// refresh failures of any of them. Keys serialize alphabetically, as upstream's do.
pub fn encode(provider_ids: &[String], state: &ApiState) -> Vec<u8> {
    let providers = provider_ids
        .iter()
        .filter_map(|id| {
            let descriptors = state.limit_descriptors.get(id)?;
            let snapshot = state.snapshots.get(id)?;
            Some((id.clone(), WireProvider::new(snapshot, descriptors, state)))
        })
        .collect();
    let errors = provider_ids
        .iter()
        .filter_map(|id| {
            state.errors.get(id).map(|message| WireError {
                message: message.clone(),
                provider_id: id.clone(),
            })
        })
        .collect();
    let envelope = WireEnvelope {
        errors,
        generated_at: iso(state.generated_at),
        providers,
        schema: SCHEMA,
    };
    serde_json::to_vec(&envelope).unwrap_or_else(|_| {
        format!(r#"{{"errors":[],"providers":{{}},"schema":"{SCHEMA}"}}"#).into_bytes()
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireEnvelope {
    errors: Vec<WireError>,
    generated_at: String,
    providers: BTreeMap<String, WireProvider>,
    schema: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireError {
    message: String,
    provider_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireProvider {
    display_name: String,
    expires_at: String,
    fetched_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<String>,
    resources: BTreeMap<String, WireResource>,
    stale: bool,
}

impl WireProvider {
    fn new(
        snapshot: &ProviderSnapshot,
        descriptors: &[WidgetDescriptor],
        state: &ApiState,
    ) -> Self {
        let interval = Duration::from_std(state.refresh_interval).unwrap_or(Duration::minutes(5));
        let expiry = snapshot.refreshed_at + interval;
        let mut resources = BTreeMap::new();
        for descriptor in descriptors {
            let Some(line) = snapshot.line(&descriptor.metric_label) else {
                continue;
            };
            for resource in &descriptor.limit_resources {
                if let Some(value) = WireResource::new(resource, line) {
                    resources.insert(resource.key.clone(), value);
                }
            }
        }
        Self {
            display_name: snapshot.display_name.clone(),
            expires_at: iso(expiry),
            fetched_at: iso(snapshot.refreshed_at),
            plan: snapshot.plan.clone(),
            resources,
            stale: state.generated_at >= expiry,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireResource {
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "optional_number"
    )]
    available: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    estimated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<Vec<String>>,
    kind: LimitResourceKind,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "optional_number"
    )]
    limit: Option<f64>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "optional_number"
    )]
    remaining: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resets_at: Option<String>,
    unit: String,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "optional_number"
    )]
    used: Option<f64>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "optional_number"
    )]
    utilization: Option<f64>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "optional_number"
    )]
    window_seconds: Option<f64>,
}

impl WireResource {
    fn new(resource: &LimitResourceDescriptor, line: &MetricLine) -> Option<Self> {
        let mut wire = Self {
            available: None,
            estimated: None,
            expires_at: None,
            kind: resource.kind,
            limit: None,
            remaining: None,
            resets_at: None,
            unit: progress_unit(line).unwrap_or_else(|| resource.unit.clone()),
            used: None,
            utilization: None,
            window_seconds: None,
        };
        match (&resource.source, line) {
            (
                LimitResourceSource::Progress | LimitResourceSource::ProgressOrValue { .. },
                MetricLine::Progress(progress),
            ) => wire.apply_progress(progress, resource),
            (
                LimitResourceSource::Value { kind, label }
                | LimitResourceSource::ProgressOrValue { kind, label },
                MetricLine::Values(values),
            ) => {
                let metric = values.values.iter().find(|value| {
                    value.kind == *kind
                        && label
                            .as_ref()
                            .is_none_or(|label| value.label.as_ref() == Some(label))
                })?;
                match resource.kind {
                    LimitResourceKind::Balance => wire.available = Some(metric.number),
                    LimitResourceKind::Consumption => wire.used = Some(metric.number),
                }
                if !values.expiries_at.is_empty() {
                    let mut expiries = values.expiries_at.clone();
                    expiries.sort();
                    wire.expires_at = Some(expiries.into_iter().map(iso).collect());
                }
                wire.estimated = (resource.estimated || metric.estimated).then_some(true);
            }
            _ => return None,
        }
        Some(wire)
    }

    fn apply_progress(&mut self, progress: &ProgressLine, resource: &LimitResourceDescriptor) {
        let limit = progress.limit.max(0.0);
        let used = progress.used.max(0.0);
        match resource.kind {
            LimitResourceKind::Consumption => self.used = Some(used),
            LimitResourceKind::Balance => self.available = Some(used),
        }
        self.limit = Some(limit);
        self.remaining = Some((limit - used).max(0.0));
        self.utilization = (limit > 0.0).then(|| used / limit);
        self.resets_at = progress.resets_at.map(iso);
        self.window_seconds = progress
            .period_duration_ms
            .map(|milliseconds| milliseconds as f64 / 1000.0);
        self.estimated = resource.estimated.then_some(true);
    }
}

/// A progress row carries its runtime unit, which can vary by plan (a count meter names its own
/// suffix); the descriptor's unit is the fallback for value rows.
fn progress_unit(line: &MetricLine) -> Option<String> {
    let MetricLine::Progress(progress) = line else {
        return None;
    };
    match &progress.format {
        ProgressFormat::Percent => Some("percent".into()),
        ProgressFormat::Dollars => Some("usd".into()),
        ProgressFormat::Count { suffix } => {
            Some(suffix.trim().to_string()).filter(|suffix| !suffix.is_empty())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};

    use chrono::{DateTime, Utc};
    use serde_json::Value;
    use uc_core::{MetricKind, MetricValue, Provider};

    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn fetched_at() -> DateTime<Utc> {
        at("2026-07-13T01:39:30.000Z")
    }

    pub(crate) fn state(
        snapshot: ProviderSnapshot,
        descriptors: Vec<WidgetDescriptor>,
    ) -> ApiState {
        let id = snapshot.provider_id.clone();
        ApiState {
            enabled_ordered_ids: vec![id.clone()],
            known_ids: BTreeSet::from([id.clone()]),
            snapshots: HashMap::from([(id.clone(), snapshot)]),
            limit_descriptors: HashMap::from([(id, descriptors)]),
            errors: HashMap::new(),
            generated_at: at("2026-07-13T01:40:00.000Z"),
            refresh_interval: std::time::Duration::from_secs(300),
        }
    }

    fn envelope(ids: &[&str], state: &ApiState) -> Value {
        let ids: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
        serde_json::from_slice(&encode(&ids, state)).unwrap()
    }

    fn codex() -> Provider {
        Provider::new("codex", "Codex")
    }

    fn codex_descriptors() -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::percent("codex.session", &codex(), "Session", None, None)
                .exporting_progress("session", "percent"),
            WidgetDescriptor::combined("codex.credits", &codex(), "Credits", None, false)
                .exporting_limit(
                    "credits",
                    LimitResourceKind::Balance,
                    "credits",
                    LimitResourceSource::Value {
                        kind: MetricKind::Count,
                        label: Some("credits".into()),
                    },
                    false,
                )
                .exporting_limit(
                    "creditValue",
                    LimitResourceKind::Balance,
                    "usd",
                    LimitResourceSource::Value {
                        kind: MetricKind::Dollars,
                        label: None,
                    },
                    false,
                ),
        ]
    }

    fn codex_snapshot() -> ProviderSnapshot {
        let session = MetricLine::progress("Session", 42.0, 100.0, ProgressFormat::Percent)
            .resets_at(Some(at("2026-07-13T06:00:00.000Z")))
            .period_ms(Some(18_000_000))
            .color("#ff0000");
        let credits = MetricLine::values(
            "Credits",
            vec![
                MetricValue::dollars(32.84),
                MetricValue::count(821.0, "credits"),
            ],
        );
        let trend = uc_core::ChartLine {
            label: "Usage Trend".into(),
            points: vec![],
            note: Some("UI only".into()),
        };
        ProviderSnapshot::make(
            &codex(),
            Some("Pro 20x".into()),
            vec![session.into(), credits.into(), trend.into()],
            fetched_at(),
        )
    }

    #[test]
    fn envelope_carries_raw_scalars_and_freshness_without_presentation() {
        let state = state(codex_snapshot(), codex_descriptors());
        let root = envelope(&["codex"], &state);
        let provider = &root["providers"]["codex"];
        let session = &provider["resources"]["session"];
        let credits = &provider["resources"]["credits"];
        assert_eq!(root["schema"], "openusage.limits.v1");
        assert_eq!(root["generatedAt"], "2026-07-13T01:40:00.000Z");
        assert_eq!(provider["plan"], "Pro 20x");
        assert_eq!(provider["fetchedAt"], "2026-07-13T01:39:30.000Z");
        assert_eq!(provider["expiresAt"], "2026-07-13T01:44:30.000Z");
        assert_eq!(provider["stale"], false);
        assert_eq!(session["kind"], "consumption");
        assert_eq!(session["unit"], "percent");
        assert_eq!(session["used"], 42);
        assert_eq!(session["limit"], 100);
        assert_eq!(session["remaining"], 58);
        assert!((session["utilization"].as_f64().unwrap() - 0.42).abs() < 1e-9);
        assert_eq!(session["windowSeconds"], 18_000);
        assert_eq!(session["resetsAt"], "2026-07-13T06:00:00.000Z");
        assert!(session.get("color").is_none());
        assert_eq!(credits["kind"], "balance");
        assert_eq!(credits["available"], 821);
        assert_eq!(provider["resources"]["creditValue"]["available"], 32.84);
        assert_eq!(provider["resources"].as_object().unwrap().len(), 3);
    }

    #[test]
    fn serialized_keys_are_sorted_like_upstream() {
        let state = state(codex_snapshot(), codex_descriptors());
        let text = String::from_utf8(encode(&["codex".to_string()], &state)).unwrap();
        assert!(text.starts_with(r#"{"errors":[],"generatedAt":"#));
        assert!(text.contains(r#""displayName":"Codex","expiresAt":"#));
        assert!(
            text.contains(
                r#""session":{"kind":"consumption","limit":100,"remaining":58,"resetsAt":"#
            )
        );
    }

    #[test]
    fn an_entry_turns_stale_once_its_interval_has_passed() {
        let mut state = state(codex_snapshot(), codex_descriptors());
        state.generated_at = at("2026-07-13T01:44:30.000Z");
        assert_eq!(
            envelope(&["codex"], &state)["providers"]["codex"]["stale"],
            true
        );
    }

    #[test]
    fn failures_are_reported_beside_a_missing_snapshot() {
        let mut state = state(codex_snapshot(), codex_descriptors());
        state.snapshots.clear();
        assert!(
            envelope(&["codex"], &state)["providers"]
                .as_object()
                .unwrap()
                .is_empty()
        );
        state.errors.insert("codex".into(), "Not logged in".into());
        let root = envelope(&["codex"], &state);
        assert_eq!(root["errors"][0]["providerId"], "codex");
        assert_eq!(root["errors"][0]["message"], "Not logged in");
    }

    #[test]
    fn flexible_consumption_never_invents_a_limit() {
        let claude = Provider::new("claude", "Claude");
        let extra = WidgetDescriptor::bounded_dollars(
            "claude.extra",
            &claude,
            "Extra Usage",
            Some("Extra usage spent"),
            100.0,
            None,
            None,
        )
        .exporting_limit(
            "extraUsage",
            LimitResourceKind::Consumption,
            "usd",
            LimitResourceSource::ProgressOrValue {
                kind: MetricKind::Dollars,
                label: None,
            },
            false,
        );
        let snapshot = ProviderSnapshot::make(
            &claude,
            None,
            vec![MetricLine::values("Extra usage spent", vec![MetricValue::dollars(12.5)]).into()],
            fetched_at(),
        );
        let root = envelope(&["claude"], &state(snapshot, vec![extra]));
        let resource = &root["providers"]["claude"]["resources"]["extraUsage"];
        assert_eq!(resource["used"], 12.5);
        assert_eq!(resource["unit"], "usd");
        for absent in ["limit", "remaining", "utilization"] {
            assert!(resource.get(absent).is_none(), "{absent}");
        }
    }

    #[test]
    fn progress_units_follow_the_runtime_format() {
        let cursor = Provider::new("cursor", "Cursor");
        let total = WidgetDescriptor::percent(
            "cursor.usage",
            &cursor,
            "Total Usage",
            Some("Total usage"),
            None,
        )
        .exporting_progress("totalUsage", "percent");
        let snapshot = ProviderSnapshot::make(
            &cursor,
            None,
            vec![
                MetricLine::progress(
                    "Total usage",
                    37.0,
                    750.0,
                    ProgressFormat::Count {
                        suffix: " requests ".into(),
                    },
                )
                .into(),
            ],
            fetched_at(),
        );
        let root = envelope(&["cursor"], &state(snapshot, vec![total]));
        let resource = &root["providers"]["cursor"]["resources"]["totalUsage"];
        assert_eq!(resource["unit"], "requests");
        assert_eq!(resource["limit"], 750);
    }

    #[test]
    fn value_resources_carry_sorted_expiries_and_estimates() {
        let resets = WidgetDescriptor::values(
            "codex.rateLimitResets",
            &codex(),
            "Rate Limit Resets",
            None,
            Some(MetricKind::Count),
            Some("available"),
            false,
            Some("resets"),
            true,
        )
        .exporting_limit(
            "rateLimitResets",
            LimitResourceKind::Balance,
            "resets",
            LimitResourceSource::Value {
                kind: MetricKind::Count,
                label: Some("available".into()),
            },
            false,
        );
        let mut line = MetricLine::values(
            "Rate Limit Resets",
            vec![MetricValue::count(2.0, "available").estimated(true)],
        );
        line.expiries_at = vec![at("2026-08-02T00:00:00Z"), at("2026-07-20T00:00:00Z")];
        let snapshot = ProviderSnapshot::make(&codex(), None, vec![line.into()], fetched_at());
        let root = envelope(&["codex"], &state(snapshot, vec![resets]));
        let resource = &root["providers"]["codex"]["resources"]["rateLimitResets"];
        assert_eq!(resource["available"], 2);
        assert_eq!(resource["estimated"], true);
        assert_eq!(
            resource["expiresAt"],
            serde_json::json!(["2026-07-20T00:00:00.000Z", "2026-08-02T00:00:00.000Z"])
        );
    }

    #[test]
    fn a_label_mismatch_or_wrong_row_shape_is_omitted() {
        let descriptors = codex_descriptors();
        let snapshot = ProviderSnapshot::make(
            &codex(),
            None,
            vec![
                MetricLine::values("Session", vec![MetricValue::new(40.0, MetricKind::Percent)])
                    .into(),
                MetricLine::values("Credits", vec![MetricValue::count(5.0, "tokens")]).into(),
            ],
            fetched_at(),
        );
        let root = envelope(&["codex"], &state(snapshot, descriptors));
        let resources = root["providers"]["codex"]["resources"].as_object().unwrap();
        assert!(resources.is_empty(), "{resources:?}");
    }
}
