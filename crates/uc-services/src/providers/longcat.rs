//! LongCat: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct LongCat;

#[async_trait]
impl Service for LongCat {
    fn id(&self) -> &'static str {
        "longcat"
    }

    fn name(&self) -> &'static str {
        "LongCat"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "LongCat is not supported yet.",
        ))
    }
}
