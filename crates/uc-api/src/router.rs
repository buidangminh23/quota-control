//! Routing for the read-only local API (upstream `LocalUsageAPI.respond`), kept pure so it is tested
//! without sockets; `server` is only the transport.

use crate::state::ApiState;
use crate::{limits, usage};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: Option<Vec<u8>>,
    pub content_type: Option<String>,
}

impl Response {
    pub fn ok(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            body: Some(body),
            content_type: None,
        }
    }

    pub fn file(body: Vec<u8>, content_type: String) -> Self {
        Self {
            status: 200,
            body: Some(body),
            content_type: Some(content_type),
        }
    }

    pub fn error(status: u16, code: &str) -> Self {
        Self {
            status,
            body: Some(format!(r#"{{"error":"{code}"}}"#).into_bytes()),
            content_type: None,
        }
    }

    pub fn busy() -> Self {
        Self::error(503, "server_busy")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppRequest {
    Command { name: String, query: Option<String> },
    Asset(String),
}

pub fn app_request(method: &str, path: &str) -> Option<AppRequest> {
    if method != "GET" {
        return None;
    }
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    let segments: Vec<&str> = route
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    match segments.as_slice() {
        ["web", rest @ ..] => {
            if rest.iter().any(|part| *part == ".." || part.contains('\\')) {
                return None;
            }
            Some(AppRequest::Asset(if rest.is_empty() {
                "index.html".to_string()
            } else {
                rest.join("/")
            }))
        }
        ["v1", "app", command] => Some(AppRequest::Command {
            name: (*command).to_string(),
            query: query
                .split('&')
                .find_map(|pair| pair.strip_prefix("q="))
                .and_then(percent_decode),
        }),
        _ => None,
    }
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                index += 3;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

pub fn respond(method: &str, path: &str, state: &ApiState) -> Response {
    if method == "OPTIONS" {
        return Response {
            status: 204,
            body: None,
            content_type: None,
        };
    }
    let path = path.split('?').next().unwrap_or_default();
    let segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let route = match segments.as_slice() {
        ["v1", "limits"] => Route::Limits(None),
        ["v1", "limits", token] => Route::Limits(Some(token)),
        ["v1", "usage"] => Route::Usage(None),
        ["v1", "usage", token] => Route::Usage(Some(token)),
        _ => return Response::error(404, "not_found"),
    };
    if method != "GET" {
        return Response::error(405, "method_not_allowed");
    }
    let ids = match route {
        Route::Limits(Some(token)) | Route::Usage(Some(token)) => {
            let ids = state.matching_ids(token);
            if ids.is_empty() {
                return Response::error(404, "provider_not_found");
            }
            ids
        }
        Route::Limits(None) | Route::Usage(None) => state.enabled_ordered_ids.clone(),
    };
    match route {
        Route::Limits(_) => Response::ok(limits::encode(&ids, state)),
        Route::Usage(_) => {
            let snapshots: Vec<_> = ids
                .iter()
                .filter_map(|id| state.snapshots.get(id))
                .collect();
            Response::ok(usage::encode(&snapshots))
        }
    }
}

enum Route<'a> {
    Limits(Option<&'a str>),
    Usage(Option<&'a str>),
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};

    use chrono::Utc;
    use serde_json::Value;
    use uc_core::{MetricLine, ProgressFormat, Provider, ProviderSnapshot, WidgetDescriptor};

    use super::*;

    fn snapshot(id: &str, used: f64) -> ProviderSnapshot {
        ProviderSnapshot::make(
            &Provider::new(id, id),
            None,
            vec![MetricLine::progress("Session", used, 100.0, ProgressFormat::Percent).into()],
            Utc::now(),
        )
    }

    fn state() -> ApiState {
        let ids = ["codex@b", "claude@a", "codex@a", "claude-local"];
        let descriptors = |id: &str| {
            vec![
                WidgetDescriptor::percent(
                    format!("{id}.session"),
                    &Provider::new(id, id),
                    "Session",
                    None,
                    None,
                )
                .exporting_progress("session", "percent"),
            ]
        };
        ApiState {
            enabled_ordered_ids: vec!["codex@b".into(), "claude-local".into(), "claude@a".into()],
            known_ids: ids.iter().map(|id| id.to_string()).collect::<BTreeSet<_>>(),
            snapshots: HashMap::from([
                ("codex@b".to_string(), snapshot("codex@b", 10.0)),
                ("claude@a".to_string(), snapshot("claude@a", 20.0)),
                ("claude-local".to_string(), snapshot("claude-local", 0.0)),
            ]),
            limit_descriptors: ["codex@b", "claude@a", "codex@a"]
                .into_iter()
                .map(|id| (id.to_string(), descriptors(id)))
                .collect(),
            errors: HashMap::new(),
            generated_at: Utc::now(),
            refresh_interval: std::time::Duration::from_secs(300),
        }
    }

    fn json(response: &Response) -> Value {
        serde_json::from_slice(response.body.as_ref().unwrap()).unwrap()
    }

    #[test]
    fn collections_serve_enabled_providers_in_dashboard_order() {
        let state = state();
        let usage = respond("GET", "/v1/usage", &state);
        assert_eq!(usage.status, 200);
        let ids: Vec<_> = json(&usage)
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["providerId"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, ["codex@b", "claude-local", "claude@a"]);
        let limits = json(&respond("GET", "/v1/limits?pretty=1", &state));
        let providers = limits["providers"].as_object().unwrap();
        assert_eq!(
            providers.keys().collect::<Vec<_>>(),
            ["claude@a", "codex@b"]
        );
    }

    #[test]
    fn a_family_token_names_every_card_and_an_unknown_token_is_404() {
        let state = state();
        let family = json(&respond("GET", "/v1/limits/codex", &state));
        assert_eq!(family["providers"].as_object().unwrap().len(), 1);
        let usage = json(&respond("GET", "/v1/usage/codex", &state));
        assert_eq!(usage.as_array().unwrap().len(), 1);
        let missing = respond("GET", "/v1/usage/codex@a", &state);
        assert_eq!(
            (missing.status, json(&missing)),
            (200, serde_json::json!([]))
        );
        let unknown = respond("GET", "/v1/limits/cursor", &state);
        assert_eq!(unknown.status, 404);
        assert_eq!(json(&unknown)["error"], "provider_not_found");
    }

    #[test]
    fn app_requests_decode_the_command_and_its_query() {
        let command = |name: &str, query: Option<&str>| {
            Some(AppRequest::Command {
                name: name.into(),
                query: query.map(String::from),
            })
        };
        assert_eq!(
            app_request(
                "GET",
                "/v1/app/public_feed?q=%7B%22name%22%3A%22claudeResets%22%7D"
            ),
            command("public_feed", Some(r#"{"name":"claudeResets"}"#))
        );
        assert_eq!(
            app_request("GET", "/v1/app/prices"),
            command("prices", None)
        );
        assert_eq!(
            app_request("GET", "/v1/app/x?q=%E1%BA%A1%2"),
            command("x", None)
        );
        assert_eq!(app_request("POST", "/v1/app/prices"), None);
        assert_eq!(app_request("GET", "/v1/app"), None);
        assert_eq!(app_request("GET", "/v1/app/a/b"), None);
        assert_eq!(app_request("GET", "/v1/limits"), None);
        assert_eq!(
            app_request("GET", "/web"),
            Some(AppRequest::Asset("index.html".into()))
        );
        assert_eq!(
            app_request("GET", "/web/assets/index-1.js?v=2"),
            Some(AppRequest::Asset("assets/index-1.js".into()))
        );
        assert_eq!(app_request("GET", "/web/../secret"), None);
        assert_eq!(app_request("GET", r"/web/a\b"), None);
    }

    #[test]
    fn methods_routes_and_preflight_follow_the_contract() {
        let state = state();
        assert_eq!(respond("POST", "/v1/limits", &state).status, 405);
        assert_eq!(
            json(&respond("DELETE", "/v1/usage/codex", &state))["error"],
            "method_not_allowed"
        );
        assert_eq!(respond("POST", "/v2/limits", &state).status, 404);
        assert_eq!(json(&respond("GET", "/", &state))["error"], "not_found");
        assert_eq!(respond("GET", "/v1/limits/codex/extra", &state).status, 404);
        assert_eq!(respond("GET", "/v1/app/usage_summary", &state).status, 404);
        let preflight = respond("OPTIONS", "/anything", &state);
        assert_eq!((preflight.status, preflight.body), (204, None));
        assert_eq!(respond("GET", "//v1//limits", &state).status, 200);
    }
}
