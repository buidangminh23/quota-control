//! Command Code: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct CommandCode;

#[async_trait]
impl Service for CommandCode {
    fn id(&self) -> &'static str {
        "commandcode"
    }

    fn name(&self) -> &'static str {
        "Command Code"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "Command Code is not supported yet.",
        ))
    }
}
