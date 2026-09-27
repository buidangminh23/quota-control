//! ai&: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct AiAnd;

#[async_trait]
impl Service for AiAnd {
    fn id(&self) -> &'static str {
        "aiand"
    }

    fn name(&self) -> &'static str {
        "ai&"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "ai& is not supported yet.",
        ))
    }
}
