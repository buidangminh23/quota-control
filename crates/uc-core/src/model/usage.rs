//! Provider-neutral daily usage carriers shared by every spend-tracking provider.
//!
//! Port of upstream `DailyUsageSeries.swift`. Key names match the upstream `Codable` output
//! (`totalTokens`, `costUSD`, …).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
}

impl TokenUsage {
    pub fn saturating_add(self, other: Self) -> Self {
        Self {
            input_tokens: self.input_tokens.saturating_add(other.input_tokens),
            output_tokens: self.output_tokens.saturating_add(other.output_tokens),
            cached_input_tokens: self
                .cached_input_tokens
                .saturating_add(other.cached_input_tokens),
            cache_creation_input_tokens: self
                .cache_creation_input_tokens
                .saturating_add(other.cache_creation_input_tokens),
        }
    }
}

/// One calendar day of usage: `date` is a `yyyy-MM-dd` key in the local time zone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DailyUsageEntry {
    pub date: String,
    #[serde(rename = "totalTokens")]
    pub total_tokens: i64,
    #[serde(
        rename = "tokenUsage",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub token_usage: Option<TokenUsage>,
    #[serde(rename = "costUSD", default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DailyUsageSeries {
    pub daily: Vec<DailyUsageEntry>,
}

/// One raw slug inside a grouped `ModelUsageEntry`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelUsageVariant {
    pub model: String,
    #[serde(rename = "totalTokens")]
    pub total_tokens: i64,
    #[serde(rename = "costUSD", default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// Token/cost totals for one model before a period collapses it into a spend row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelUsageEntry {
    pub model: String,
    #[serde(rename = "totalTokens")]
    pub total_tokens: i64,
    #[serde(
        rename = "tokenUsage",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub token_usage: Option<TokenUsage>,
    #[serde(rename = "costUSD", default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variants: Option<Vec<ModelUsageVariant>>,
}

impl ModelUsageEntry {
    pub const UNATTRIBUTED_MODEL_NAME: &'static str = "Unattributed";
    pub const OTHER_MODEL_NAME: &'static str = "Other";

    pub fn new(model: impl Into<String>, total_tokens: i64, cost_usd: Option<f64>) -> Self {
        Self {
            model: model.into(),
            total_tokens,
            token_usage: None,
            cost_usd,
            variants: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DailyModelUsageEntry {
    pub date: String,
    pub models: Vec<ModelUsageEntry>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelUsageSeries {
    pub daily: Vec<DailyModelUsageEntry>,
}

/// Day key (`yyyy-MM-dd`) → model names. A `BTreeMap`/`BTreeSet` keeps serialization deterministic.
pub type ModelsByDay = BTreeMap<String, BTreeSet<String>>;

/// The presentation-free daily history retained on a provider snapshot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderUsageHistory {
    pub series: DailyUsageSeries,
    #[serde(
        rename = "modelUsage",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub model_usage: Option<ModelUsageSeries>,
    #[serde(rename = "unknownModelsByDay", default)]
    pub unknown_models_by_day: ModelsByDay,
    #[serde(
        rename = "fallbackPricingModelsByDay",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub fallback_pricing_models_by_day: Option<ModelsByDay>,
}

/// A period-scoped, UI-ready model breakdown attached to a spend row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelUsageBreakdown {
    #[serde(rename = "totalTokens")]
    pub total_tokens: i64,
    #[serde(
        rename = "tokenUsage",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub token_usage: Option<TokenUsage>,
    #[serde(
        rename = "totalCostUSD",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub total_cost_usd: Option<f64>,
    pub models: Vec<ModelUsageEntry>,
    #[serde(rename = "sourceNote")]
    pub source_note: String,
}

/// Shared result shape of the native log scanners.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LogUsageScan {
    pub series: DailyUsageSeries,
    pub model_usage: Option<ModelUsageSeries>,
    pub unknown_models_by_day: ModelsByDay,
    pub fallback_pricing_models_by_day: Option<ModelsByDay>,
}

impl From<LogUsageScan> for ProviderUsageHistory {
    fn from(scan: LogUsageScan) -> Self {
        Self {
            series: scan.series,
            model_usage: scan.model_usage,
            unknown_models_by_day: scan.unknown_models_by_day,
            fallback_pricing_models_by_day: scan.fallback_pricing_models_by_day,
        }
    }
}
