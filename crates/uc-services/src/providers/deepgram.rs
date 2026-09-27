//! Deepgram: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct Deepgram;

#[async_trait]
impl Service for Deepgram {
    fn id(&self) -> &'static str {
        "deepgram"
    }

    fn name(&self) -> &'static str {
        "Deepgram"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "Deepgram is not supported yet.",
        ))
    }
}
