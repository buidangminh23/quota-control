//! AWS Bedrock: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct Bedrock;

#[async_trait]
impl Service for Bedrock {
    fn id(&self) -> &'static str {
        "bedrock"
    }

    fn name(&self) -> &'static str {
        "AWS Bedrock"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "AWS Bedrock is not supported yet.",
        ))
    }
}
