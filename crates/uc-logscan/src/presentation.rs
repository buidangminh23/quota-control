use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use uc_core::*;

use crate::{LogScanner, LogSource, ScanReport};

const SOURCE_NOTE: &str = "All local sessions on this machine; not account-scoped. Input includes cached tokens; output is model-generated. Costs estimate API-equivalent usage using bundled 2026-07-02 prices, not subscription charges.";
const TOKEN_LABELS: [(&str, &str); 3] = [
    ("inputTokens", "Input Tokens"),
    ("outputTokens", "Output Tokens"),
    ("cachedInputTokens", "Cached Input Tokens"),
];

pub fn history_descriptors(provider: &Provider) -> Vec<WidgetDescriptor> {
    let mut descriptors = vec![WidgetDescriptor::usage_trend(provider).exporting_history(
        UsageHistoryScope::MachineLocal,
        true,
        SOURCE_NOTE,
    )];
    descriptors.extend(WidgetDescriptor::spend_tiles(provider, Some(SOURCE_NOTE)));
    for (suffix, label) in TOKEN_LABELS {
        let mut descriptor = WidgetDescriptor::values(
            format!("{}.{suffix}", provider.id),
            provider,
            label,
            None,
            Some(MetricKind::Count),
            None,
            true,
            None,
            false,
        );
        descriptor.template.value_tooltip_note = Some(format!("Today. {SOURCE_NOTE}"));
        descriptors.push(descriptor);
    }
    descriptors
}

pub fn append_history(snapshot: &mut ProviderSnapshot, report: ScanReport, now: DateTime<Utc>) {
    let labels = [
        "Today",
        "Yesterday",
        "Last 30 Days",
        "Usage Trend",
        "Input Tokens",
        "Output Tokens",
        "Cached Input Tokens",
    ];
    snapshot
        .lines
        .retain(|line| !labels.contains(&line.label()));
    snapshot.usage_history = None;
    for warning in &report.warnings {
        warn(snapshot, warning);
    }
    if report.records == 0 {
        return;
    }
    let today = report
        .today
        .unwrap_or_else(|| now.with_timezone(&chrono::Local).date_naive());
    let history: ProviderUsageHistory = report.usage.into();
    let uncertain = report.incomplete || report.skipped_lines > 0;
    for (label, first, last) in [
        ("Today", today, today),
        (
            "Yesterday",
            today - Duration::days(1),
            today - Duration::days(1),
        ),
        ("Last 30 Days", today - Duration::days(29), today),
    ] {
        let first = first.to_string();
        let last = last.to_string();
        let days: Vec<_> = history
            .series
            .daily
            .iter()
            .filter(|d| d.date >= first && d.date <= last)
            .collect();
        let total = days
            .iter()
            .fold(0_i64, |a, d| a.saturating_add(d.total_tokens));
        let cost = days.iter().try_fold(0.0, |a, d| d.cost_usd.map(|b| a + b));
        let usage = days.iter().try_fold(TokenUsage::default(), |a, d| {
            d.token_usage.map(|b| a.saturating_add(b))
        });
        let mut values = Vec::new();
        if !uncertain && let Some(cost) = cost {
            values.push(MetricValue::dollars(cost).estimated(true));
        }
        values.push(MetricValue::count(total as f64, "tokens").estimated(uncertain));
        let mut models: BTreeMap<String, ModelUsageEntry> = BTreeMap::new();
        if let Some(series) = &history.model_usage {
            for day in series
                .daily
                .iter()
                .filter(|d| d.date >= first && d.date <= last)
            {
                for row in &day.models {
                    models
                        .entry(row.model.clone())
                        .and_modify(|entry| {
                            entry.total_tokens =
                                entry.total_tokens.saturating_add(row.total_tokens);
                            entry.cost_usd = entry.cost_usd.zip(row.cost_usd).map(|(a, b)| a + b);
                            entry.token_usage = entry
                                .token_usage
                                .zip(row.token_usage)
                                .map(|(a, b)| a.saturating_add(b));
                        })
                        .or_insert_with(|| row.clone());
                }
            }
        }
        let unknown: BTreeSet<_> = history
            .unknown_models_by_day
            .iter()
            .filter(|(d, _)| **d >= first && **d <= last)
            .flat_map(|(_, models)| models.iter().cloned())
            .collect();
        let mut line = MetricLine::values(label, values);
        line.unknown_models = unknown.into_iter().collect();
        line.model_breakdown = Some(ModelUsageBreakdown {
            total_tokens: total,
            token_usage: usage,
            total_cost_usd: if uncertain { None } else { cost },
            models: models.into_values().collect(),
            source_note: SOURCE_NOTE.into(),
        });
        snapshot.lines.push(line.into());
        if label == "Today"
            && let Some(usage) = usage
        {
            for ((_, name), count) in TOKEN_LABELS.into_iter().zip([
                usage.input_tokens,
                usage.output_tokens,
                usage.cached_input_tokens,
            ]) {
                snapshot.lines.push(
                    MetricLine::values(
                        name,
                        vec![MetricValue::count(count as f64, "tokens").estimated(uncertain)],
                    )
                    .into(),
                );
            }
        }
    }
    let daily: BTreeMap<_, _> = history
        .series
        .daily
        .iter()
        .map(|d| (d.date.as_str(), d.total_tokens))
        .collect();
    let points = (0..30)
        .map(|index| {
            let date = today - Duration::days(29 - index);
            let total = daily.get(date.to_string().as_str()).copied().unwrap_or(0);
            MetricChartPoint {
                value: total as f64,
                label: date.format("%b %d").to_string(),
                value_label: Some(format!("{total} tokens")),
            }
        })
        .collect();
    snapshot.lines.push(
        ChartLine {
            label: "Usage Trend".into(),
            points,
            note: Some(SOURCE_NOTE.into()),
        }
        .into(),
    );
    if !history.unknown_models_by_day.is_empty() {
        warn(
            snapshot,
            "Some local usage has no supported model price; affected cost totals are unavailable.",
        );
    }
    snapshot.usage_history = Some(history);
}

fn warn(snapshot: &mut ProviderSnapshot, notice: &str) {
    match &mut snapshot.warning {
        Some(existing) if !existing.contains(notice) => {
            existing.push(' ');
            existing.push_str(notice);
        }
        None => snapshot.warning = Some(notice.into()),
        _ => {}
    }
}

pub struct HistoryRuntime {
    inner: Arc<dyn ProviderRuntime>,
    scanner: LogScanner,
    clock: Clock,
}

impl HistoryRuntime {
    pub fn new(inner: Arc<dyn ProviderRuntime>, scanner: LogScanner) -> Self {
        Self {
            inner,
            scanner,
            clock: system_clock(),
        }
    }

    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }
}

#[async_trait]
impl ProviderRuntime for HistoryRuntime {
    fn provider(&self) -> &Provider {
        self.inner.provider()
    }

    fn widget_descriptors(&self) -> Vec<WidgetDescriptor> {
        let mut descriptors = self.inner.widget_descriptors();
        if self.provider().id.contains('@') {
            return descriptors;
        }
        for descriptor in history_descriptors(self.provider()) {
            if !descriptors.iter().any(|d| d.id == descriptor.id) {
                descriptors.push(descriptor);
            }
        }
        descriptors
    }

    fn allows_cached_local_history(&self) -> bool {
        self.inner.allows_cached_local_history()
    }

    async fn refresh(&self, context: RefreshContext) -> ProviderSnapshot {
        let now = (self.clock)();
        let mut snapshot = self.inner.refresh(context).await;
        if !self.allows_cached_local_history() || self.provider().id.contains('@') {
            return snapshot;
        }
        let scanner = self.scanner.clone();
        match tokio::task::spawn_blocking(move || scanner.scan(now)).await {
            Ok(report) => append_history(&mut snapshot, report, now),
            Err(_) => warn(&mut snapshot, "Local history scanner failed."),
        }
        snapshot
    }

    async fn has_local_credentials(&self) -> bool {
        self.inner.has_local_credentials().await
    }
}

pub struct LocalHistoryRuntime {
    provider: Provider,
    scanner: LogScanner,
    clock: Clock,
}

impl LocalHistoryRuntime {
    pub fn new(source: LogSource) -> Self {
        let (id, name, icon) = match source {
            LogSource::Claude => ("claude-local", "Claude Local Usage", "claude"),
            LogSource::Codex => ("codex-local", "Codex Local Usage", "codex"),
        };
        let mut provider = Provider::new(id, name);
        provider.icon = icon.into();
        Self {
            provider,
            scanner: LogScanner::from_environment(source),
            clock: system_clock(),
        }
    }

    pub fn with_scanner(mut self, scanner: LogScanner) -> Self {
        assert_eq!(self.scanner.source, scanner.source);
        self.scanner = scanner;
        self
    }

    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }
}

#[async_trait]
impl ProviderRuntime for LocalHistoryRuntime {
    fn provider(&self) -> &Provider {
        &self.provider
    }

    fn widget_descriptors(&self) -> Vec<WidgetDescriptor> {
        history_descriptors(&self.provider)
    }

    async fn refresh(&self, _context: RefreshContext) -> ProviderSnapshot {
        let now = (self.clock)();
        let mut snapshot = ProviderSnapshot::make(
            &self.provider,
            Some("Machine-local history".into()),
            Vec::new(),
            now,
        );
        let scanner = self.scanner.clone();
        match tokio::task::spawn_blocking(move || scanner.scan(now)).await {
            Ok(report) => {
                let incomplete = report.incomplete;
                let empty = report.records == 0;
                append_history(&mut snapshot, report, now);
                if empty {
                    snapshot.lines.push(
                        MetricLine::badge(
                            "Status",
                            if incomplete {
                                "Local history unavailable"
                            } else {
                                "No local usage in the last 30 days"
                            },
                        )
                        .into(),
                    );
                }
            }
            Err(_) => {
                return ProviderSnapshot::error_message(
                    &self.provider,
                    "Local history scanner failed.",
                    Some(ErrorCategory::Other),
                );
            }
        }
        snapshot
    }

    async fn has_local_credentials(&self) -> bool {
        self.scanner.options.roots.iter().any(|root| {
            std::fs::symlink_metadata(root).is_ok_and(|m| m.is_dir() && !super::linked(&m))
        })
    }
}
