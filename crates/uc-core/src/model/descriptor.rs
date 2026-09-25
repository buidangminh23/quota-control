//! A provider metric's identity and presentation template.
//!
//! Port of upstream `WidgetDescriptor.swift`, `WidgetDescriptor+Factories.swift`,
//! `LimitResourceDescriptor.swift` and `UsageHistoryDescriptor.swift`. Upstream keeps a full
//! `WidgetData` as the descriptor's `sample`; here only the template fields the factories set are
//! carried (`WidgetTemplate`), because the popup computes the rest from live snapshot lines.

use serde::{Deserialize, Serialize};

use super::metric::MetricKind;
use super::provider::Provider;

/// How a session-window meter tells a not-yet-started window from an in-flight one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionStartSignal {
    /// Zero usage is the fresh signal (Antigravity, OpenCode).
    ZeroUsage,
    /// A missing reset date is the fresh signal (Claude).
    MissingResetDate,
}

/// Structural presentation metadata for one widget. Template numbers are never shown: a row without
/// real data renders the no-data marker instead.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetTemplate {
    pub title: String,
    pub kind: MetricKind,
    /// `None` → unbounded (number row).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count_suffix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_noun: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unbounded_value_word: Option<String>,
    /// Which of a `values` row's numbers this widget renders. `None` renders every value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_kind: Option<MetricKind>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_usage_period: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tray_suffix: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shows_reset_expiries: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_start_signal: Option<SessionStartSignal>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_chart: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_tooltip_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period_duration_ms: Option<i64>,
}

impl WidgetTemplate {
    pub fn new(title: impl Into<String>, kind: MetricKind, limit: Option<f64>) -> Self {
        Self {
            title: title.into(),
            kind,
            limit,
            count_suffix: None,
            value_prefix: None,
            limit_noun: None,
            unbounded_value_word: None,
            selection_kind: None,
            is_usage_period: false,
            tray_suffix: None,
            shows_reset_expiries: false,
            session_start_signal: None,
            is_chart: false,
            value_tooltip_note: None,
            info_note: None,
            period_duration_ms: None,
        }
    }
}

/// Stable, machine-facing metadata for one resource exported by `/v1/limits`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitResourceDescriptor {
    pub key: String,
    pub kind: LimitResourceKind,
    pub unit: String,
    pub source: LimitResourceSource,
    #[serde(default)]
    pub estimated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LimitResourceKind {
    Consumption,
    Balance,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LimitResourceSource {
    Progress,
    Value {
        kind: MetricKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// A provider may report the same consumption as bounded progress or as an uncapped scalar.
    ProgressOrValue {
        kind: MetricKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
}

/// Whether a provider's daily spend history may be added across machines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UsageHistoryScope {
    MachineLocal,
    AccountWide,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageHistoryDescriptor {
    pub scope: UsageHistoryScope,
    pub estimated_cost: bool,
    pub source_note: String,
}

/// One widget a provider can feed: its stable id, the snapshot line it reads, and its template.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetDescriptor {
    /// e.g. `claude.session`.
    pub id: String,
    pub provider_id: String,
    /// The `MetricLine::label` this widget reads from a snapshot.
    pub metric_label: String,
    pub template: WidgetTemplate,
    /// False for tiles the tray can't render as a value (the Usage Trend chart).
    pub pinnable: bool,
    /// True only for the Today / Yesterday / Last 30 Days spend tiles.
    pub is_spend_tile: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limit_resources: Vec<LimitResourceDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_resource: Option<UsageHistoryDescriptor>,
}

impl WidgetDescriptor {
    fn make(id: impl Into<String>, provider: &Provider, metric_label: String, template: WidgetTemplate) -> Self {
        Self {
            id: id.into(),
            provider_id: provider.id.clone(),
            metric_label,
            template,
            pinnable: true,
            is_spend_tile: false,
            limit_resources: Vec::new(),
            history_resource: None,
        }
    }

    /// Bounded 0–100% meter (session/weekly-style quotas).
    pub fn percent(
        id: impl Into<String>,
        provider: &Provider,
        title: &str,
        metric_label: Option<&str>,
        session_start_signal: Option<SessionStartSignal>,
    ) -> Self {
        let mut template = WidgetTemplate::new(title, MetricKind::Percent, Some(100.0));
        template.session_start_signal = session_start_signal;
        Self::make(id, provider, metric_label.unwrap_or(title).to_string(), template)
    }

    /// Bounded dollar meter whose subtitle reads "$<limit> <limitNoun>".
    pub fn bounded_dollars(
        id: impl Into<String>,
        provider: &Provider,
        title: &str,
        metric_label: Option<&str>,
        limit: f64,
        limit_noun: Option<&str>,
        value_word: Option<&str>,
    ) -> Self {
        let mut template = WidgetTemplate::new(title, MetricKind::Dollars, Some(limit));
        template.limit_noun = limit_noun.map(str::to_string);
        template.unbounded_value_word = value_word.map(str::to_string);
        Self::make(id, provider, metric_label.unwrap_or(title).to_string(), template)
    }

    /// Bounded count meter (e.g. requests per billing cycle).
    pub fn bounded_count(
        id: impl Into<String>,
        provider: &Provider,
        title: &str,
        metric_label: Option<&str>,
        limit: f64,
        suffix: &str,
        period_duration_ms: Option<i64>,
    ) -> Self {
        let mut template = WidgetTemplate::new(title, MetricKind::Count, Some(limit));
        template.count_suffix = Some(suffix.to_string());
        template.period_duration_ms = period_duration_ms;
        Self::make(id, provider, metric_label.unwrap_or(title).to_string(), template)
    }

    /// Unbounded numeric row backed by a provider `values` line.
    #[allow(clippy::too_many_arguments)]
    pub fn values(
        id: impl Into<String>,
        provider: &Provider,
        title: &str,
        metric_label: Option<&str>,
        selection_kind: Option<MetricKind>,
        value_word: Option<&str>,
        is_usage_period: bool,
        tray_suffix: Option<&str>,
        shows_reset_expiries: bool,
    ) -> Self {
        let kind = selection_kind.unwrap_or(MetricKind::Dollars);
        let mut template = WidgetTemplate::new(title, kind, None);
        template.unbounded_value_word = value_word.map(str::to_string);
        template.selection_kind = selection_kind;
        template.is_usage_period = is_usage_period;
        template.tray_suffix = tray_suffix.map(str::to_string);
        template.shows_reset_expiries = shows_reset_expiries;
        Self::make(id, provider, metric_label.unwrap_or(title).to_string(), template)
    }

    /// Combined tile reading "$4.08 · 1.2M tokens": every value of a `values` row, joined.
    pub fn combined(
        id: impl Into<String>,
        provider: &Provider,
        title: &str,
        metric_label: Option<&str>,
        is_usage_period: bool,
    ) -> Self {
        Self::values(id, provider, title, metric_label, None, None, is_usage_period, None, false)
    }

    /// The Today / Yesterday / Last 30 Days tiles every spend-tracking provider exposes.
    pub fn spend_tiles(provider: &Provider, value_tooltip_note: Option<&str>) -> Vec<Self> {
        [("today", "Today"), ("yesterday", "Yesterday"), ("last30", "Last 30 Days")]
            .into_iter()
            .map(|(suffix, title)| {
                let mut descriptor =
                    Self::combined(format!("{}.{suffix}", provider.id), provider, title, None, true);
                descriptor.template.value_tooltip_note = value_tooltip_note.map(str::to_string);
                descriptor.is_spend_tile = true;
                descriptor
            })
            .collect()
    }

    /// Unbounded dollar balance with a custom trailing word (e.g. "$1,503.00 left").
    pub fn dollar_balance(
        id: impl Into<String>,
        provider: &Provider,
        title: &str,
        metric_label: Option<&str>,
        value_word: &str,
    ) -> Self {
        let mut template = WidgetTemplate::new(title, MetricKind::Dollars, None);
        template.unbounded_value_word = Some(value_word.to_string());
        Self::make(id, provider, metric_label.unwrap_or(title).to_string(), template)
    }

    /// Unbounded count resolved from a provider `badge` line (e.g. Grok pay-as-you-go).
    pub fn badge(id: impl Into<String>, provider: &Provider, title: &str, metric_label: Option<&str>) -> Self {
        let template = WidgetTemplate::new(title, MetricKind::Count, None);
        Self::make(id, provider, metric_label.unwrap_or(title).to_string(), template)
    }

    /// The Usage Trend row: a day-by-day token sparkline backed by a provider `chart` line.
    pub fn usage_trend(provider: &Provider) -> Self {
        let mut template = WidgetTemplate::new("Usage Trend", MetricKind::Count, None);
        template.is_chart = true;
        let mut descriptor =
            Self::make(format!("{}.trend", provider.id), provider, "Usage Trend".to_string(), template);
        descriptor.pinnable = false;
        descriptor
    }

    /// Adds one scalar to the public limits contract without changing the widget or mapper.
    pub fn exporting_limit(
        mut self,
        key: &str,
        kind: LimitResourceKind,
        unit: &str,
        source: LimitResourceSource,
        estimated: bool,
    ) -> Self {
        self.limit_resources.push(LimitResourceDescriptor {
            key: key.to_string(),
            kind,
            unit: unit.to_string(),
            source,
            estimated,
        });
        self
    }

    /// Shorthand for the common consumption/percent-style export read from a `progress` line.
    pub fn exporting_progress(self, key: &str, unit: &str) -> Self {
        self.exporting_limit(key, LimitResourceKind::Consumption, unit, LimitResourceSource::Progress, false)
    }

    /// Classifies the provider's normalized daily history beside its other machine-facing exports.
    pub fn exporting_history(mut self, scope: UsageHistoryScope, estimated_cost: bool, source_note: &str) -> Self {
        self.history_resource =
            Some(UsageHistoryDescriptor { scope, estimated_cost, source_note: source_note.to_string() });
        self
    }

    pub fn title(&self) -> &str {
        &self.template.title
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude() -> Provider {
        Provider::new("claude", "Claude")
    }

    #[test]
    fn spend_tiles_share_ids_and_flags() {
        let tiles = WidgetDescriptor::spend_tiles(&claude(), None);
        let ids: Vec<_> = tiles.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["claude.today", "claude.yesterday", "claude.last30"]);
        assert!(tiles.iter().all(|t| t.is_spend_tile && t.template.is_usage_period));
        assert_eq!(tiles[2].metric_label, "Last 30 Days");
    }

    #[test]
    fn usage_trend_is_not_pinnable() {
        let trend = WidgetDescriptor::usage_trend(&claude());
        assert!(!trend.pinnable);
        assert!(trend.template.is_chart);
        assert_eq!(trend.id, "claude.trend");
    }

    #[test]
    fn percent_defaults_metric_label_to_title() {
        let session = WidgetDescriptor::percent(
            "claude.session",
            &claude(),
            "Session",
            None,
            Some(SessionStartSignal::MissingResetDate),
        )
        .exporting_progress("session", "percent");
        assert_eq!(session.metric_label, "Session");
        assert_eq!(session.template.limit, Some(100.0));
        assert_eq!(session.limit_resources[0].key, "session");
        let json = serde_json::to_value(&session).unwrap();
        assert_eq!(json["template"]["sessionStartSignal"], "missingResetDate");
    }
}
