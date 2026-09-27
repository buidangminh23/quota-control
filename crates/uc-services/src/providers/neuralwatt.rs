//! Neuralwatt: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct Neuralwatt;

#[async_trait]
impl Service for Neuralwatt {
    fn id(&self) -> &'static str {
        "neuralwatt"
    }

    fn name(&self) -> &'static str {
        "Neuralwatt"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "Neuralwatt is not supported yet.",
        ))
    }
}
