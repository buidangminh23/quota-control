//! Shared foundation for Usage Control: the normalized metric vocabulary every provider produces,
//! the provider runtime contract, HTTP plumbing, formatting, dates, paths and log redaction.
//!
//! This is a port of the model layer of OpenUsage (MIT, Robin Ebers). File-level docs name the
//! upstream Swift source each module mirrors.

pub mod error;
pub mod fallback;
pub mod format;
pub mod http;
pub mod model;
pub mod paths;
pub mod redact;
pub mod runtime;
pub mod time;

pub use error::{ErrorCategory, ProviderError, SimpleProviderError};
pub use http::{HttpClient, HttpError, HttpRequest, HttpResponse, ProxyConfig, ReqwestHttpClient, SharedHttpClient};
pub use model::*;
pub use runtime::{Clock, ProviderRuntime, RefreshContext, fixed_clock, load_blocking, system_clock, with_deadline};

/// The app's display name.
pub const APP_NAME: &str = "Usage Control";
/// The version shown in the popup footer.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
