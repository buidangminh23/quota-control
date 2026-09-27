//! Atlas Cloud: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct AtlasCloud;

#[async_trait]
impl Service for AtlasCloud {
    fn id(&self) -> &'static str {
        "atlascloud"
    }

    fn name(&self) -> &'static str {
        "Atlas Cloud"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "Atlas Cloud is not supported yet.",
        ))
    }
}
