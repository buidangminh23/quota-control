//! JetBrains AI: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct JetBrains;

#[async_trait]
impl Service for JetBrains {
    fn id(&self) -> &'static str {
        "jetbrains"
    }

    fn name(&self) -> &'static str {
        "JetBrains AI"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "JetBrains AI is not supported yet.",
        ))
    }
}
