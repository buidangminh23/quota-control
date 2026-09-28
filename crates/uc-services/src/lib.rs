//! Quota readers for AI services other than Claude and Codex.
//!
//! A [`Service`] reads either the login its own CLI, IDE or desktop app already saved on this
//! computer, or an API key the user saved in Quota Control; it calls the service's usage endpoint
//! and maps the answer into metric lines. [`ServiceRuntime`] turns one service plus one credential
//! into a card, and [`detect`] and [`runtimes`] find every card this computer has.
//!
//! Services never write another app's files and never spend another app's refresh token in a way
//! that would sign it out: a renewed access token lives only in the card's memory.

mod catalog;
mod providers;
mod runtime;
mod service;
pub mod signin;
pub mod support;
#[cfg(test)]
pub(crate) mod testing;

pub use catalog::{
    Detected, DetectedSource, RETIRED_SERVICES, ServiceInfo, detect, identity_hash, login_card_id,
    runtimes, service, service_infos, services,
};
pub use runtime::{CredentialSource, ServiceRuntime};
pub use service::{
    ApiKeyHelp, Connection, FetchContext, KeyFormat, Login, MAX_KEY_LENGTH, Memo, Reading, Roots,
    Secret, Service,
};
