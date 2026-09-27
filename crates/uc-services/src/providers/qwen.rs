//! Qwen Cloud: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct Qwen;

#[async_trait]
impl Service for Qwen {
    fn id(&self) -> &'static str {
        "qwen"
    }

    fn name(&self) -> &'static str {
        "Qwen Cloud"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "Qwen Cloud is not supported yet.",
        ))
    }
}
