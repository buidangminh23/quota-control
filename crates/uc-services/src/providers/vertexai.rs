//! Vertex AI: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct VertexAi;

#[async_trait]
impl Service for VertexAi {
    fn id(&self) -> &'static str {
        "vertexai"
    }

    fn name(&self) -> &'static str {
        "Vertex AI"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "Vertex AI is not supported yet.",
        ))
    }
}
