//! SiliconFlow: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct SiliconFlow;

#[async_trait]
impl Service for SiliconFlow {
    fn id(&self) -> &'static str {
        "siliconflow"
    }

    fn name(&self) -> &'static str {
        "SiliconFlow"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "SiliconFlow is not supported yet.",
        ))
    }
}
