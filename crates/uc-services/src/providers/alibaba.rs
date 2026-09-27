//! Alibaba Model Studio: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct Alibaba;

#[async_trait]
impl Service for Alibaba {
    fn id(&self) -> &'static str {
        "alibaba"
    }

    fn name(&self) -> &'static str {
        "Alibaba Model Studio"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "Alibaba Model Studio is not supported yet.",
        ))
    }
}
