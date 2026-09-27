//! TypeSafe: a card that explains why TypeSafe billing cannot be read.
//!
//! Nothing is read on Windows, macOS or Linux: no file, no environment variable, no key and no
//! network request. The reference reader fetches the billing page's HTML, searches up to 60
//! JavaScript chunks for a deployment-specific `Next-Action` and starts that discovery over after
//! a 404, which does not fit in four read requests without page-action state scraped elsewhere. A
//! refresh therefore only reports that TypeSafe billing is not available.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct TypeSafe;

const MESSAGE: &str = "TypeSafe billing requires a browser session and a dynamically discovered page action that this reader cannot safely obtain.";

#[async_trait]
impl Service for TypeSafe {
    fn id(&self) -> &'static str {
        "typesafe"
    }

    fn name(&self) -> &'static str {
        "TypeSafe"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(MESSAGE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at};

    #[tokio::test]
    async fn reports_constraint_without_accessing_credentials_or_network() {
        let http = Scripted::new();
        let scope = context_at(&http, serde_json::json!({}), chrono::Utc::now());
        let error = TypeSafe.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, uc_core::ErrorCategory::NotAvailable);
        assert_eq!(error.message, MESSAGE);
        assert!(http.requests().is_empty());
        let dir = tempfile::tempdir().unwrap();
        assert!(
            TypeSafe
                .discover(&crate::service::Roots::under(dir.path()))
                .is_empty()
        );
        assert!(TypeSafe.connection().api_key.is_none());
    }
}
