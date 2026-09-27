//! LLM Proxy: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct LlmProxy;

#[async_trait]
impl Service for LlmProxy {
    fn id(&self) -> &'static str {
        "llmproxy"
    }

    fn name(&self) -> &'static str {
        "LLM Proxy"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "LLM Proxy is not supported yet.",
        ))
    }
}
