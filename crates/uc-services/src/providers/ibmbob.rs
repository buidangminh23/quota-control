//! IBM Bob: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct IbmBob;

#[async_trait]
impl Service for IbmBob {
    fn id(&self) -> &'static str {
        "ibmbob"
    }

    fn name(&self) -> &'static str {
        "IBM Bob"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "IBM Bob is not supported yet.",
        ))
    }
}
