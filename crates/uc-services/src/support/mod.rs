//! Helpers every service uses: sending requests, reading loose JSON, building metric lines,
//! reading app state from SQLite or the credential store, renewing OAuth tokens in memory.

pub mod apps;
pub mod endpoint;
pub mod http;
pub mod jwt;
pub mod keyring;
pub mod lines;
pub mod oauth;
pub mod sqlite;
pub mod value;
