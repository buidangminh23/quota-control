//! Moonshot: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct Moonshot;

#[async_trait]
impl Service for Moonshot {
    fn id(&self) -> &'static str {
        "moonshot"
    }

    fn name(&self) -> &'static str {
        "Moonshot"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "Moonshot is not supported yet.",
        ))
    }
}
