//! Perplexity: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct Perplexity;

#[async_trait]
impl Service for Perplexity {
    fn id(&self) -> &'static str {
        "perplexity"
    }

    fn name(&self) -> &'static str {
        "Perplexity"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "Perplexity is not supported yet.",
        ))
    }
}
