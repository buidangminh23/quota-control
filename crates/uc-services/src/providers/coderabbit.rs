//! CodeRabbit: a card with no usage to show, because none can be read without running
//! CodeRabbit's CLI.
//!
//! The reference implementation gets CodeRabbit's usage only by running `coderabbit usage`, and
//! CodeRabbit's CLI and API key documentation describes signing in but no read-only HTTP usage
//! endpoint and no credential file layout. So nothing is discovered on Windows, macOS or Linux,
//! the card takes no key, and a refresh sends no request: it reports that the usage needs the CLI.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};
use crate::support::http;

pub(crate) struct CodeRabbit;

const MESSAGE: &str = "CodeRabbit usage currently requires launching its CLI; no verified read-only HTTP usage endpoint is available.";

#[async_trait]
impl Service for CodeRabbit {
    fn id(&self) -> &'static str {
        "coderabbit"
    }

    fn name(&self) -> &'static str {
        "CodeRabbit"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(http::not_available(MESSAGE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Roots;
    use crate::testing::{Scripted, context_at};
    use chrono::Utc;
    use serde_json::json;
    use uc_core::ErrorCategory;

    #[tokio::test]
    async fn nothing_is_read_or_sent_and_a_refresh_says_the_cli_is_needed() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), Utc::now());
        let error = CodeRabbit.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::NotAvailable);
        assert_eq!(error.message, MESSAGE);
        assert!(http.requests().is_empty());
        let dir = tempfile::tempdir().unwrap();
        assert!(CodeRabbit.discover(&Roots::under(dir.path())).is_empty());
        assert!(CodeRabbit.connection().api_key.is_none());
    }
}
