//! LiteLLM: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct LiteLLM;

#[async_trait]
impl Service for LiteLLM {
    fn id(&self) -> &'static str {
        "litellm"
    }

    fn name(&self) -> &'static str {
        "LiteLLM"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "LiteLLM is not supported yet.",
        ))
    }
}
