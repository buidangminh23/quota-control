//! Cerebras: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct Cerebras;

#[async_trait]
impl Service for Cerebras {
    fn id(&self) -> &'static str {
        "cerebras"
    }

    fn name(&self) -> &'static str {
        "Cerebras"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "Cerebras is not supported yet.",
        ))
    }
}
