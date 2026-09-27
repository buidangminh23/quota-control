//! Vercel AI Gateway: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct Vercel;

#[async_trait]
impl Service for Vercel {
    fn id(&self) -> &'static str {
        "vercel"
    }

    fn name(&self) -> &'static str {
        "Vercel AI Gateway"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "Vercel AI Gateway is not supported yet.",
        ))
    }
}
