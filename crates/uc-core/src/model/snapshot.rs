//! Port of upstream `ProviderSnapshot.swift`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::metric::{BadgeLine, MetricLine};
use super::provider::Provider;
use super::usage::ProviderUsageHistory;
use crate::error::{ErrorCategory, ProviderError};

/// Latest normalized output for one provider refresh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderSnapshot {
    #[serde(rename = "providerID")]
    pub provider_id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    pub lines: Vec<MetricLine>,
    #[serde(rename = "refreshedAt")]
    pub refreshed_at: DateTime<Utc>,
    /// Raw normalized daily history used to build spend rows. Always this machine's own history.
    #[serde(rename = "usageHistory", default, skip_serializing_if = "Option::is_none")]
    pub usage_history: Option<ProviderUsageHistory>,
    /// A soft notice carried on a successful snapshot (the header's amber triangle).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    /// Set only on error snapshots: a stable, non-PII bucket for the failure.
    #[serde(rename = "errorCategory", default, skip_serializing_if = "Option::is_none")]
    pub error_category: Option<ErrorCategory>,
}

impl ProviderSnapshot {
    /// The success-path constructor: derives id and name from the provider.
    pub fn make(provider: &Provider, plan: Option<String>, lines: Vec<MetricLine>, refreshed_at: DateTime<Utc>) -> Self {
        Self {
            provider_id: provider.id.clone(),
            display_name: provider.display_name.clone(),
            plan,
            lines,
            refreshed_at,
            usage_history: None,
            warning: None,
            error_category: None,
        }
    }

    pub fn with_history(mut self, history: Option<ProviderUsageHistory>) -> Self {
        self.usage_history = history;
        self
    }

    pub fn with_warning(mut self, warning: Option<String>) -> Self {
        self.warning = warning;
        self
    }

    /// Build an error snapshot from a typed provider error: the badge text is the error's
    /// user-facing message, the category comes from the error itself.
    pub fn error(provider: &Provider, error: &dyn ProviderError) -> Self {
        Self::error_message(provider, error.to_string(), Some(error.category()))
    }

    pub fn error_message(provider: &Provider, message: impl Into<String>, category: Option<ErrorCategory>) -> Self {
        Self {
            provider_id: provider.id.clone(),
            display_name: provider.display_name.clone(),
            plan: None,
            lines: vec![MetricLine::Badge(BadgeLine {
                label: MetricLine::ERROR_BADGE_LABEL.to_string(),
                text: message.into(),
                color_hex: Some("#EF4444".to_string()),
                subtitle: None,
            })],
            refreshed_at: Utc::now(),
            usage_history: None,
            warning: None,
            error_category: category.or(Some(ErrorCategory::Other)),
        }
    }

    pub fn line(&self, label: &str) -> Option<&MetricLine> {
        self.lines.iter().find(|line| line.label() == label)
    }

    /// True when this snapshot is a provider-level error (its only line is the error badge).
    pub fn is_error(&self) -> bool {
        self.lines.iter().any(MetricLine::is_error)
    }

    /// The user-facing error text of an error snapshot.
    pub fn error_text(&self) -> Option<&str> {
        self.lines.iter().find_map(|line| match line {
            MetricLine::Badge(badge) if badge.label == MetricLine::ERROR_BADGE_LABEL => Some(badge.text.as_str()),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SimpleProviderError;

    #[test]
    fn error_snapshot_carries_category_and_badge() {
        let provider = Provider::new("codex", "Codex");
        let error = SimpleProviderError::new(ErrorCategory::NotLoggedIn, "Not logged in");
        let snapshot = ProviderSnapshot::error(&provider, &error);
        assert!(snapshot.is_error());
        assert_eq!(snapshot.error_text(), Some("Not logged in"));
        assert_eq!(snapshot.error_category, Some(ErrorCategory::NotLoggedIn));
        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["providerID"], "codex");
        assert_eq!(json["errorCategory"], "not_logged_in");
    }
}
