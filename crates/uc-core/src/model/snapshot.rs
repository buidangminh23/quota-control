//! Port of upstream `ProviderSnapshot.swift`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::metric::{BadgeLine, MetricLine};
use super::provider::Provider;
use super::usage::ProviderUsageHistory;
use crate::error::{ErrorCategory, ProviderError};

/// What the provider says about the plan's current paid period.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "basis")]
pub enum PlanTerm {
    /// The provider states when the paid period ends (ChatGPT's login carries it).
    #[serde(rename = "stated")]
    Stated {
        #[serde(rename = "endsAt")]
        ends_at: DateTime<Utc>,
        /// When the provider last confirmed that date.
        #[serde(rename = "checkedAt", default, skip_serializing_if = "Option::is_none")]
        checked_at: Option<DateTime<Utc>>,
    },
    /// A legacy monthly estimate, bounded by its last confirmation. A subscription start alone
    /// does not establish the billing cadence or current paid entitlement.
    #[serde(rename = "monthlyFrom")]
    MonthlyFrom {
        #[serde(rename = "startedAt")]
        started_at: DateTime<Utc>,
        #[serde(rename = "checkedAt", default, skip_serializing_if = "Option::is_none")]
        checked_at: Option<DateTime<Utc>>,
    },
}

/// Latest normalized output for one provider refresh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderSnapshot {
    #[serde(rename = "providerID")]
    pub provider_id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(rename = "planCheckedAt", default, skip_serializing_if = "Option::is_none")]
    pub plan_checked_at: Option<DateTime<Utc>>,
    /// The plan's paid period, when the account states or implies one.
    #[serde(rename = "planTerm", default, skip_serializing_if = "Option::is_none")]
    pub plan_term: Option<PlanTerm>,
    /// The signed-in account's email as the provider reports it, when its API names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
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
            plan_checked_at: None,
            plan_term: None,
            account: None,
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

    pub fn with_plan_term(mut self, term: Option<PlanTerm>) -> Self {
        self.plan_term = term;
        self
    }

    pub fn with_account(mut self, account: Option<String>) -> Self {
        self.account = account;
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
            plan_checked_at: None,
            plan_term: None,
            account: None,
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
        assert!(json.get("planTerm").is_none());
    }

    #[test]
    fn plan_terms_serialize_with_their_basis_and_round_trip() {
        use chrono::TimeZone;
        let provider = Provider::new("codex", "Codex");
        let ends_at = Utc.with_ymd_and_hms(2026, 10, 17, 1, 56, 39).unwrap();
        let started_at = Utc.with_ymd_and_hms(2026, 7, 31, 3, 40, 9).unwrap();
        for (term, expected) in [
            (
                PlanTerm::Stated {
                    ends_at,
                    checked_at: None,
                },
                serde_json::json!({"basis": "stated", "endsAt": "2026-10-17T01:56:39Z"}),
            ),
            (
                PlanTerm::MonthlyFrom { started_at, checked_at: None },
                serde_json::json!({"basis": "monthlyFrom", "startedAt": "2026-07-31T03:40:09Z"}),
            ),
        ] {
            let snapshot = ProviderSnapshot::make(&provider, None, Vec::new(), ends_at)
                .with_plan_term(Some(term.clone()));
            let json = serde_json::to_value(&snapshot).unwrap();
            assert_eq!(json["planTerm"], expected);
            let back: ProviderSnapshot = serde_json::from_value(json).unwrap();
            assert_eq!(back.plan_term, Some(term));
        }
        let cached: ProviderSnapshot = serde_json::from_value(serde_json::json!({
            "providerID": "codex", "displayName": "Codex", "lines": [], "refreshedAt": "2026-09-27T00:00:00Z"
        }))
        .unwrap();
        assert_eq!(cached.plan_term, None);
        assert_eq!(cached.plan_checked_at, None);
    }

    #[test]
    fn subscription_confirmation_survives_serialization_and_legacy_terms_load() {
        use chrono::TimeZone;
        let checked_at = Utc.with_ymd_and_hms(2026, 10, 9, 10, 0, 0).unwrap();
        let term: PlanTerm = serde_json::from_value(serde_json::json!({
            "basis": "monthlyFrom", "startedAt": "2026-07-31T03:40:09Z"
        })).unwrap();
        assert!(matches!(term, PlanTerm::MonthlyFrom { checked_at: None, .. }));
        let mut snapshot = ProviderSnapshot::make(&Provider::new("claude", "Claude"), Some("Free".into()), vec![], checked_at);
        snapshot.plan_checked_at = Some(checked_at);
        let value = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(value["planCheckedAt"], "2026-10-09T10:00:00Z");
        let restored: ProviderSnapshot = serde_json::from_value(value).unwrap();
        assert_eq!(restored.plan_checked_at, Some(checked_at));
    }
}
