//! DeepInfra: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct DeepInfra;

#[async_trait]
impl Service for DeepInfra {
    fn id(&self) -> &'static str {
        "deepinfra"
    }

    fn name(&self) -> &'static str {
        "DeepInfra"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "DeepInfra is not supported yet.",
        ))
    }
}
