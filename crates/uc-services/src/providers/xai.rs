//! xAI: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct Xai;

#[async_trait]
impl Service for Xai {
    fn id(&self) -> &'static str {
        "xai"
    }

    fn name(&self) -> &'static str {
        "xAI"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "xAI is not supported yet.",
        ))
    }
}
