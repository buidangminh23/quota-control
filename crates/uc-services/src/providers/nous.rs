//! Nous Portal: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct Nous;

#[async_trait]
impl Service for Nous {
    fn id(&self) -> &'static str {
        "nous"
    }

    fn name(&self) -> &'static str {
        "Nous Portal"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "Nous Portal is not supported yet.",
        ))
    }
}
