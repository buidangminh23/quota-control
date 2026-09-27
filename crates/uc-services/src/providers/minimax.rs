//! MiniMax: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct MiniMax;

#[async_trait]
impl Service for MiniMax {
    fn id(&self) -> &'static str {
        "minimax"
    }

    fn name(&self) -> &'static str {
        "MiniMax"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "MiniMax is not supported yet.",
        ))
    }
}
