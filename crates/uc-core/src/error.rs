//! Port of upstream `ErrorCategory.swift`: a stable, machine-readable bucket for refresh failures.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorCategory {
    #[serde(rename = "not_logged_in")]
    NotLoggedIn,
    /// A previously-valid credential went bad (expired / revoked / conflicting session).
    #[serde(rename = "auth_expired")]
    AuthExpired,
    /// Auth is structurally wrong rather than stale.
    #[serde(rename = "auth_invalid")]
    AuthInvalid,
    /// Local credential material exists, but could not be read.
    #[serde(rename = "credential_access")]
    CredentialAccess,
    /// The request never completed (transport / connection failure).
    #[serde(rename = "network")]
    Network,
    /// A response came back but could not be parsed.
    #[serde(rename = "decoding")]
    Decoding,
    #[serde(rename = "http_4xx")]
    Http4xx,
    #[serde(rename = "http_5xx")]
    Http5xx,
    #[serde(rename = "rate_limited")]
    RateLimited,
    /// Usage data is legitimately unavailable for this account or plan.
    #[serde(rename = "not_available")]
    NotAvailable,
    #[serde(rename = "other")]
    Other,
}

impl ErrorCategory {
    /// Classify a non-2xx HTTP status.
    pub fn http(status: u16) -> Self {
        match status {
            429 => ErrorCategory::RateLimited,
            400..=499 => ErrorCategory::Http4xx,
            500..=599 => ErrorCategory::Http5xx,
            _ => ErrorCategory::Other,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCategory::NotLoggedIn => "not_logged_in",
            ErrorCategory::AuthExpired => "auth_expired",
            ErrorCategory::AuthInvalid => "auth_invalid",
            ErrorCategory::CredentialAccess => "credential_access",
            ErrorCategory::Network => "network",
            ErrorCategory::Decoding => "decoding",
            ErrorCategory::Http4xx => "http_4xx",
            ErrorCategory::Http5xx => "http_5xx",
            ErrorCategory::RateLimited => "rate_limited",
            ErrorCategory::NotAvailable => "not_available",
            ErrorCategory::Other => "other",
        }
    }
}

/// An error that knows its own telemetry bucket. Its `Display` is the friendly, user-facing text.
pub trait ProviderError: std::error::Error + Send + Sync {
    fn category(&self) -> ErrorCategory;
}

/// A provider error built from a category and a message, for failures without a dedicated enum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimpleProviderError {
    pub category: ErrorCategory,
    pub message: String,
}

impl SimpleProviderError {
    pub fn new(category: ErrorCategory, message: impl Into<String>) -> Self {
        Self { category, message: message.into() }
    }
}

impl std::fmt::Display for SimpleProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SimpleProviderError {}

impl ProviderError for SimpleProviderError {
    fn category(&self) -> ErrorCategory {
        self.category
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_status_buckets_match_upstream() {
        assert_eq!(ErrorCategory::http(429), ErrorCategory::RateLimited);
        assert_eq!(ErrorCategory::http(401), ErrorCategory::Http4xx);
        assert_eq!(ErrorCategory::http(503), ErrorCategory::Http5xx);
        assert_eq!(ErrorCategory::http(302), ErrorCategory::Other);
    }

    #[test]
    fn serde_uses_snake_case_wire_names() {
        assert_eq!(serde_json::to_string(&ErrorCategory::Http4xx).unwrap(), "\"http_4xx\"");
        for category in [ErrorCategory::NotLoggedIn, ErrorCategory::CredentialAccess, ErrorCategory::Other] {
            assert_eq!(serde_json::to_value(category).unwrap(), category.as_str());
        }
    }
}
