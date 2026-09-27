//! llmman: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct LlmMan;

#[async_trait]
impl Service for LlmMan {
    fn id(&self) -> &'static str {
        "llmman"
    }

    fn name(&self) -> &'static str {
        "llmman"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "llmman is not supported yet.",
        ))
    }
}
