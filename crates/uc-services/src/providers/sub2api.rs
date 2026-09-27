//! sub2api: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct Sub2Api;

#[async_trait]
impl Service for Sub2Api {
    fn id(&self) -> &'static str {
        "sub2api"
    }

    fn name(&self) -> &'static str {
        "sub2api"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "sub2api is not supported yet.",
        ))
    }
}
