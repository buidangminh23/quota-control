//! The long-lived side of Usage Control: refresh orchestration, the persisted snapshot cache and
//! the documents the popup owns. Shared by the tray app and the one-shot CLI.

pub mod cache;
pub mod documents;
pub mod engine;

pub use cache::SnapshotCache;
pub use documents::{DocumentName, DocumentStore};
pub use engine::{
    Engine, EngineConfig, EngineState, HistoryRenderer, OutcomeHook, ProviderEntry, ProviderRuntimeState, RefreshOutcome,
};
