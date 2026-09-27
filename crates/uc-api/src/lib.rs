//! The machine-facing side of Quota Control: the `/v1/limits` and legacy `/v1/usage` projections,
//! the loopback HTTP server that serves them, and the one-shot `usagectl` command.
//!
//! Port of upstream `LocalLimitsAPI.swift`, `LocalUsageAPI.swift`, `LocalUsageServer.swift`,
//! `UsageReader.swift` and the `OpenUsageCLI` target. The wire formats match upstream, so tools
//! written against OpenUsage's CLI and local API read Quota Control unchanged.

mod assembly;
pub mod cli;
mod limits;
mod router;
pub mod server;
mod state;
mod usage;
mod wire;

pub use assembly::{
    ProviderSelection, ServiceCards, build_engine, provider_runtimes, provider_runtimes_with,
    select_providers, settings_list, starts_hidden,
};
pub use limits::SCHEMA;
pub use router::{Response, respond};
pub use state::ApiState;
