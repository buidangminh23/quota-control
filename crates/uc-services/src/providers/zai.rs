//! Z.ai: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct Zai;

#[async_trait]
impl Service for Zai {
    fn id(&self) -> &'static str {
        "zai"
    }

    fn name(&self) -> &'static str {
        "Z.ai"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "Z.ai is not supported yet.",
        ))
    }
}
