//! OpenAI-compatible relay: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct NewApi;

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
        Err(crate::support::http::not_available(
            "OpenAI-compatible relay is not supported yet.",
        ))
    }
}
