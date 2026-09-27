//! T3 Chat: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct T3Chat;

#[async_trait]
impl Service for T3Chat {
    fn id(&self) -> &'static str {
        "t3chat"
    }

    fn name(&self) -> &'static str {
        "T3 Chat"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "T3 Chat is not supported yet.",
        ))
    }
}
