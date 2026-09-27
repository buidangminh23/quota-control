//! Codebuff: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct Codebuff;

#[async_trait]
impl Service for Codebuff {
    fn id(&self) -> &'static str {
        "codebuff"
    }

    fn name(&self) -> &'static str {
        "Codebuff"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "Codebuff is not supported yet.",
        ))
    }
}
