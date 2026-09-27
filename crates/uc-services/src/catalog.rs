//! Every service this build knows, and the cards this computer has for them.

use std::collections::HashSet;
use std::sync::Arc;

use serde::Serialize;
use sha2::{Digest, Sha256};
use uc_accounts::{KeyRecord, KeyStore};
use uc_core::{Provider, ProviderRuntime, SharedHttpClient};

use crate::providers;
use crate::runtime::{CredentialSource, ServiceRuntime};
use crate::service::{KeyFormat, Roots, Service};
use crate::signin::Method;

/// Every service compiled into this build, in a stable order.
pub fn services() -> &'static [&'static dyn Service] {
    providers::ALL
}

pub fn service(id: &str) -> Option<&'static dyn Service> {
    services()
        .iter()
        .copied()
        .find(|service| service.id() == id)
}

/// The card-id hash of an account `identity` of `service`: 64 lowercase hex digits, like the
/// account ids of Claude and Codex.
pub fn identity_hash(service: &str, identity: &str) -> String {
    Sha256::digest(format!("{service}\n{identity}").as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn login_card_id(service: &str, identity: &str) -> String {
    format!("{service}@{}", identity_hash(service, identity))
}

/// A service as the Accounts screen lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceInfo {
    pub id: &'static str,
    pub name: &'static str,
    /// The app whose login is read from this computer, when the service reads one.
    pub login_from: Option<&'static str>,
    /// Whether an API key connects it.
    pub takes_api_key: bool,
    /// What the key field asks for, in English ("API key", "Session cookie (…)").
    pub key_label: &'static str,
    /// Whether the key is one token or a whole Cookie header.
    pub key_format: KeyFormat,
    /// Where to create a key.
    pub key_url: Option<&'static str>,
    /// Environment variables read for a key.
    pub key_env: Vec<&'static str>,
    /// Other values asked for beside the key, as `(field, English label)`.
    pub key_fields: Vec<(&'static str, &'static str)>,
    /// Whether its cards start hidden.
    pub starts_hidden: bool,
    /// The browser sign-ins it offers, in order.
    pub sign_in: Vec<Method>,
}

pub fn service_infos() -> Vec<ServiceInfo> {
    services()
        .iter()
        .map(|service| {
            let connection = service.connection();
            ServiceInfo {
                id: service.id(),
                name: service.name(),
                login_from: connection.login_from,
                takes_api_key: connection.api_key.is_some(),
                key_label: service.key_label(),
                key_format: service.key_format(),
                key_url: connection.api_key.map(|help| help.url),
                key_env: connection
                    .api_key
                    .map(|help| help.env.to_vec())
                    .unwrap_or_default(),
                key_fields: connection
                    .api_key
                    .map(|help| help.fields.to_vec())
                    .unwrap_or_default(),
                starts_hidden: service.starts_hidden(),
                sign_in: service
                    .sign_in()
                    .map(|sign_in| sign_in.methods().to_vec())
                    .unwrap_or_default(),
            }
        })
        .collect()
}

/// A card this computer has without anything saved in Quota Control: a login an app saved, or a
/// key in an environment variable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detected {
    pub id: String,
    pub service: &'static str,
    pub label: Option<String>,
    /// "Gemini CLI", or the environment variable holding the key.
    pub origin: String,
    #[serde(skip)]
    pub source: DetectedSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetectedSource {
    Login { identity_hash: String },
    Env { variable: &'static str },
}

/// Every login and environment key found under `roots`, one card per account: the first login an
/// account has wins, and a login or environment key of an account also saved here (signed in to
/// from Quota Control, or the same key) is left to the saved card.
pub fn detect(roots: &Roots, saved: &[KeyRecord]) -> Vec<Detected> {
    let saved_ids: HashSet<&str> = saved.iter().map(|record| record.id.as_str()).collect();
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for service in services() {
        for login in service.discover(roots) {
            let hash = identity_hash(service.id(), &login.identity);
            let id = format!("{}@{hash}", service.id());
            if saved_ids.contains(id.as_str()) || !seen.insert(id.clone()) {
                continue;
            }
            found.push(Detected {
                id,
                service: service.id(),
                label: login.label,
                origin: login.origin,
                source: DetectedSource::Login {
                    identity_hash: hash,
                },
            });
        }
        let Some(help) = service.connection().api_key else {
            continue;
        };
        for variable in help.env {
            let Some(key) = roots.var(variable) else {
                continue;
            };
            let id = KeyStore::key_id(service.id(), &key);
            if saved_ids.contains(id.as_str()) || !seen.insert(id.clone()) {
                continue;
            }
            found.push(Detected {
                id,
                service: service.id(),
                label: None,
                origin: (*variable).to_string(),
                source: DetectedSource::Env { variable },
            });
        }
    }
    found
}

fn card(service: &'static dyn Service, id: &str, label: Option<&str>) -> Provider {
    let name = match label.map(str::trim).filter(|label| !label.is_empty()) {
        Some(label) => format!("{} · {label}", service.name()),
        None => service.name().to_string(),
    };
    Provider::new(id, name).with_links(service.links())
}

/// The cards for `detected` logins and keys and for the keys saved in `store`.
pub fn runtimes(
    detected: &[Detected],
    store: &KeyStore,
    saved: &[KeyRecord],
    roots: &Roots,
    http: SharedHttpClient,
) -> Vec<Arc<dyn ProviderRuntime>> {
    let mut cards: Vec<Arc<dyn ProviderRuntime>> = Vec::new();
    let mut seen = HashSet::new();
    for login in detected {
        let Some(service) = service(login.service) else {
            continue;
        };
        if !seen.insert(login.id.clone()) {
            continue;
        }
        let source = match &login.source {
            DetectedSource::Login { identity_hash } => CredentialSource::Login {
                identity_hash: identity_hash.clone(),
                roots: roots.clone(),
            },
            DetectedSource::Env { variable } => CredentialSource::Env {
                variable,
                roots: roots.clone(),
            },
        };
        cards.push(Arc::new(ServiceRuntime::new(
            service,
            card(service, &login.id, login.label.as_deref()),
            source,
            http.clone(),
        )));
    }
    for record in saved {
        let Some(service) = service(&record.service) else {
            continue;
        };
        if !seen.insert(record.id.clone()) {
            continue;
        }
        let label = (record.label.trim() != service.name()).then_some(record.label.as_str());
        cards.push(Arc::new(ServiceRuntime::new(
            service,
            card(service, &record.id, label),
            CredentialSource::Saved {
                store: store.clone(),
                id: record.id.clone(),
                signed_in: record.sign_in.is_some(),
            },
            http.clone(),
        )));
    }
    cards
}
