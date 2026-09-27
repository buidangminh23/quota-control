//! TypeSafe: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct TypeSafe;

#[async_trait]
impl Service for TypeSafe {
    fn id(&self) -> &'static str {
        "typesafe"
    }

    fn name(&self) -> &'static str {
        "TypeSafe"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "TypeSafe is not supported yet.",
        ))
    }
}
