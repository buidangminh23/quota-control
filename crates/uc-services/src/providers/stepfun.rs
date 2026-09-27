//! StepFun: placeholder until its reader lands.

use async_trait::async_trait;
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Reading, Service};

pub(crate) struct StepFun;

#[async_trait]
impl Service for StepFun {
    fn id(&self) -> &'static str {
        "stepfun"
    }

    fn name(&self) -> &'static str {
        "StepFun"
    }

    fn connection(&self) -> Connection {
        Connection::default()
    }

    fn descriptors(&self, _provider: &Provider) -> Vec<WidgetDescriptor> {
        Vec::new()
    }

    async fn fetch(&self, _context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        Err(crate::support::http::not_available(
            "StepFun is not supported yet.",
        ))
    }
}
