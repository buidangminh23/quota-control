//! OpenAI-compatible relay (One API and New API): a placeholder that explains why a relay's balance
//! and spend cannot be read, because the units of its amounts cannot be verified.
//!
//! Nothing is read on Windows, macOS or Linux: no file, no environment variable, no key and no
//! network request, so the Accounts screen does not offer the relay. The relays' billing answers
//! carry `hard_limit_usd` and `total_usage`, which can hold US dollars, yuan or tokens depending
//! on the site, and neither answer says which, so showing a Balance or Used amount would risk the
//! wrong unit. The billing code is in
//! https://raw.githubusercontent.com/songquanpeng/one-api/main/controller/billing.go and
//! https://raw.githubusercontent.com/Calcium-Ion/new-api/main/controller/billing.go.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};
use crate::support::http;

pub(crate) struct NewApi;

const UNAVAILABLE: &str = "This relay's billing endpoints do not identify whether amounts are USD, CNY or tokens. Balance cannot be displayed safely.";

#[async_trait]
impl Service for NewApi {
    fn id(&self) -> &'static str {
        "newapi"
    }

    fn name(&self) -> &'static str {
        "OpenAI-compatible relay"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(http::not_available(UNAVAILABLE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at};
    use chrono::Utc;
    use serde_json::json;
    use uc_core::ErrorCategory;

    #[tokio::test]
    async fn a_refresh_reports_the_billing_as_not_available_without_sending_a_request() {
        let http = Scripted::new();
        let scope = context_at(
            &http,
            json!({"apiKey":"test","baseUrl":"https://relay.example"}),
            Utc::now(),
        );
        let error = NewApi.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::NotAvailable);
        assert_eq!(error.message, UNAVAILABLE);
        assert!(http.requests().is_empty());
        assert!(NewApi.connection().api_key.is_none());
        assert!(
            NewApi
                .descriptors(&Provider::new("newapi@x", "Relay"))
                .is_empty()
        );
    }
}
