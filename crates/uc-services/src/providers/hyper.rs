//! Charm Hyper: the Hypercredits left on a Charm Hyper account, shown as a count of `HC` rather
//! than as dollars.
//!
//! Nothing is read from disk or from a browser on Windows, macOS or Linux: the key comes from the
//! `HYPER_API_KEY` environment variable or from a Charm Hyper API key saved in Quota Control. A
//! refresh sends one request, `GET https://hyper.charm.land/v1/credits` with the key as a bearer
//! token, and shows the `balance` it answers; a balance that is not a number, or is below zero,
//! cannot be read.

use async_trait::async_trait;
use uc_core::{HttpRequest, MetricKind, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines};

pub(crate) struct Hyper;

const NAME: &str = "Charm Hyper";
const URL: &str = "https://hyper.charm.land/v1/credits";

#[async_trait]
impl Service for Hyper {
    fn id(&self) -> &'static str {
        "hyper"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["HYPER_API_KEY"],
            url: "https://hyper.charm.land",
            fields: &[],
        })
    }

    fn key_label(&self) -> &'static str {
        "Charm Hyper API key"
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![WidgetDescriptor::values(
            format!("{}.balance", provider.id),
            provider,
            "Hypercredits",
            None,
            Some(MetricKind::Count),
            Some("HC"),
            true,
            None,
            false,
        )]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The Charm Hyper API key is missing."))?;
        let body = http::json(context.http, HttpRequest::get(URL).bearer(key), NAME).await?;
        let balance = body["balance"]
            .as_f64()
            .filter(|credits| credits.is_finite() && *credits >= 0.0)
            .ok_or_else(|| http::decoding(NAME))?;
        Ok(Reading::new(
            None,
            vec![lines::count_value("Hypercredits", balance, "HC")],
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use serde_json::json;
    use uc_core::ErrorCategory;

    #[tokio::test]
    async fn the_balance_shows_as_hypercredits_and_the_key_goes_as_a_bearer_token() {
        let http = Scripted::new().on("GET", URL, 200, r#"{"balance":12.75}"#);
        let scope = context_at(&http, json!({"apiKey":"fixture"}), chrono::Utc::now());
        assert_eq!(
            Hyper.fetch(&scope.context()).await.unwrap(),
            Reading::new(None, vec![lines::count_value("Hypercredits", 12.75, "HC")])
        );
        assert_eq!(
            header(&http.requests()[0], "authorization"),
            Some("Bearer fixture")
        );
        assert_eq!(http.requests().len(), 1);
    }

    #[tokio::test]
    async fn http_errors_and_unreadable_or_negative_balances_keep_their_categories() {
        for (status, body, category) in [
            (401, "{}", ErrorCategory::AuthExpired),
            (403, "{}", ErrorCategory::AuthExpired),
            (429, "{}", ErrorCategory::RateLimited),
            (200, "{", ErrorCategory::Decoding),
            (200, r#"{"balance":-1}"#, ErrorCategory::Decoding),
            (200, r#"{"balance":"1"}"#, ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", URL, status, body);
            let scope = context_at(&http, json!({"apiKey":"fixture"}), chrono::Utc::now());
            assert_eq!(
                Hyper.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
        }
    }
}
