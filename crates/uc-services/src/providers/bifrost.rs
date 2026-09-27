//! Bifrost: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct Bifrost;

#[async_trait]
impl Service for Bifrost {
    fn id(&self) -> &'static str {
        "bifrost"
    }

    fn name(&self) -> &'static str {
        "Bifrost"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "Bifrost is not supported yet.",
        ))
    }
}
