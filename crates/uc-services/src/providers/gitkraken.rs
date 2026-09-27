//! GitKraken AI: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct GitKraken;

#[async_trait]
impl Service for GitKraken {
    fn id(&self) -> &'static str {
        "gitkraken"
    }

    fn name(&self) -> &'static str {
        "GitKraken AI"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "GitKraken AI is not supported yet.",
        ))
    }
}
