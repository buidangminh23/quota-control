use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use serde::Deserialize;
use serde::de::DeserializeOwned;
use tauri::{AppHandle, Listener, Manager};
use uc_api::{AppRequest, Response};
use uc_logscan::ledger::UsageQuery;
use uc_logscan::quality::QualityQuery;

use crate::account_commands::{self, Accounts};
use crate::insights_commands::InsightsService;
use crate::limit_resets::{self, Redemptions};
use crate::public_feeds::FeedName;
use crate::service::BackendService;
use crate::usage_commands::UsageService;

const PRICES: &str = include_str!("../../src/data/official-prices.json");

#[derive(Deserialize)]
struct QueryArgs<T> {
    query: T,
}

#[derive(Deserialize)]
struct FeedArgs {
    name: FeedName,
}

#[derive(Deserialize)]
struct DocumentArgs {
    name: String,
}

const LOGIN_RESULTS_KEPT: usize = 20;

fn login_results() -> &'static Mutex<VecDeque<serde_json::Value>> {
    static RESULTS: OnceLock<Mutex<VecDeque<serde_json::Value>>> = OnceLock::new();
    RESULTS.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn remember_login(payload: &str) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
        return;
    };
    if let Ok(mut results) = login_results().lock() {
        if results.len() >= LOGIN_RESULTS_KEPT {
            results.pop_front();
        }
        results.push_back(value);
    }
}

pub fn handler(app: AppHandle) -> uc_api::server::AppHandler {
    app.listen_any("account-login", |event| remember_login(event.payload()));
    Arc::new(move |request| {
        let app = app.clone();
        Box::pin(async move {
            match request {
                AppRequest::Action { name, body } => act(&app, &name, &body).await,
                request => respond(&app, request).await,
            }
        })
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginArgs {
    provider: String,
    language: Option<String>,
    method: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompleteArgs {
    flow_id: String,
    address: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiKeyArgs {
    service_id: String,
    key: String,
    label: Option<String>,
    fields: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Deserialize)]
struct Id {
    #[serde(
        alias = "flowId",
        alias = "accountId",
        alias = "keyId",
        alias = "cardId",
        alias = "serviceId",
        alias = "providerId",
        alias = "provider"
    )]
    id: Option<String>,
}

fn refused(message: String) -> Response {
    Response {
        status: 422,
        body: Some(
            serde_json::json!({ "error": message })
                .to_string()
                .into_bytes(),
        ),
        content_type: None,
    }
}

fn outcome<T: serde::Serialize>(value: Result<T, String>) -> Response {
    match value {
        Ok(value) => json(Ok(value)),
        Err(message) => refused(message),
    }
}

async fn act(app: &AppHandle, name: &str, body: &str) -> Option<Response> {
    let body = Query(Some(if body.trim().is_empty() {
        "{}".to_string()
    } else {
        body.to_string()
    }));
    let id = || {
        body.args::<Id>()
            .ok()
            .and_then(|args| args.id)
            .unwrap_or_default()
    };
    let response = match name {
        "begin_account_login" => match body.args::<LoginArgs>() {
            Ok(args) => outcome(
                account_commands::begin_login(
                    app.clone(),
                    &app.state::<Accounts>(),
                    args.provider,
                    args.language,
                    args.method,
                    false,
                )
                .await,
            ),
            Err(response) => response,
        },
        "complete_account_login" => match body.args::<CompleteArgs>() {
            Ok(args) => outcome(
                account_commands::complete_remote_login(
                    &app.state::<Accounts>(),
                    &args.flow_id,
                    &args.address,
                )
                .await,
            ),
            Err(response) => response,
        },
        "cancel_account_login" => {
            outcome(account_commands::cancel_account_login(app.state(), id()).await)
        }
        "remove_account" => outcome(
            account_commands::remove_account(app.clone(), app.state(), app.state(), id()).await,
        ),
        "restore_removed_logins" => outcome(
            account_commands::restore_removed_logins(app.clone(), app.state(), app.state(), id())
                .await,
        ),
        "add_api_key" => match body.args::<ApiKeyArgs>() {
            Ok(args) => outcome(
                account_commands::add_api_key(
                    app.clone(),
                    app.state(),
                    app.state(),
                    args.service_id,
                    args.label,
                    args.key,
                    args.fields,
                )
                .await,
            ),
            Err(response) => response,
        },
        "remove_api_key" => outcome(
            account_commands::remove_api_key(app.clone(), app.state(), app.state(), id()).await,
        ),
        "dismiss_detected_card" => outcome(
            account_commands::dismiss_detected_card(app.clone(), app.state(), app.state(), id())
                .await,
        ),
        "restore_dismissed_cards" => outcome(
            account_commands::restore_dismissed_cards(app.clone(), app.state(), app.state(), id())
                .await,
        ),
        "redeem_limit_reset" => outcome(
            limit_resets::redeem_limit_reset(app.state(), app.state::<Redemptions>(), id()).await,
        ),
        "refresh" => {
            let provider = body.args::<Id>().ok().and_then(|args| args.id);
            outcome(crate::commands::refresh(app.state(), provider).await)
        }
        _ => return None,
    };
    Some(response)
}

fn json<T: serde::Serialize>(value: Result<T, String>) -> Response {
    match value.and_then(|value| serde_json::to_vec(&value).map_err(|error| error.to_string())) {
        Ok(body) => Response::ok(body),
        Err(_) => Response::error(500, "unavailable"),
    }
}

async fn respond<R: tauri::Runtime>(app: &AppHandle<R>, request: AppRequest) -> Option<Response> {
    let (name, query) = match request {
        AppRequest::Asset(path) => return asset(app, &path),
        AppRequest::Command { name, query } => (name, query),
        AppRequest::Action { .. } => return None,
    };
    let request = Query(query);
    let response = match name.as_str() {
        "app_info" => json(Ok(app.state::<BackendService>().info())),
        "catalog" => json(Ok(app.state::<BackendService>().engine().catalog())),
        "engine_state" => json(Ok(app.state::<BackendService>().engine().state())),
        "load_document" => match request.args::<DocumentArgs>() {
            Ok(args) if matches!(args.name.as_str(), "settings" | "layout") => {
                json(app.state::<BackendService>().load(&args.name))
            }
            Ok(_) => Response::error(400, "bad_query"),
            Err(response) => response,
        },
        "list_accounts" => json(app.state::<Accounts>().entries()),
        "list_services" => json(Ok(app.state::<Accounts>().service_list())),
        "list_removed_logins" => json(app.state::<Accounts>().removed_logins()),
        "login_results" => json(Ok(login_results()
            .lock()
            .map(|results| results.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default())),
        "system_time_zone" => json(Ok(iana_time_zone::get_timezone()
            .ok()
            .filter(|zone| !zone.is_empty()))),
        "usage_summary" => match request.args::<QueryArgs<UsageQuery>>() {
            Ok(args) => json(app.state::<UsageService>().summary(args.query).await),
            Err(response) => response,
        },
        "usage_ledger_info" => json(app.state::<UsageService>().ledger_info().await),
        "exchange_rate" => json(Ok(app.state::<UsageService>().rate().await)),
        "context_windows" => json(app.state::<UsageService>().contexts().await),
        "model_quality" => match request.args::<QueryArgs<QualityQuery>>() {
            Ok(args) => json(app.state::<InsightsService>().quality(args.query).await),
            Err(response) => response,
        },
        "public_feed" => match request.args::<FeedArgs>() {
            Ok(args) => json(Ok(app.state::<InsightsService>().feed(args.name).await)),
            Err(response) => response,
        },
        "prices" => Response::ok(PRICES.as_bytes().to_vec()),
        _ => return None,
    };
    Some(response)
}

struct Query(Option<String>);

impl Query {
    fn args<T: DeserializeOwned>(&self) -> Result<T, Response> {
        serde_json::from_str(self.0.as_deref().unwrap_or("{}"))
            .map_err(|_| Response::error(400, "bad_query"))
    }
}

fn asset<R: tauri::Runtime>(app: &AppHandle<R>, path: &str) -> Option<Response> {
    let asset = app.asset_resolver().get(path.to_string())?;
    Some(Response::file(asset.bytes, asset.mime_type))
}

#[cfg(test)]
mod tests {
    use tauri::test::{mock_builder, mock_context, noop_assets};

    use super::*;

    fn request(command: &str, query: Option<&str>) -> AppRequest {
        AppRequest::Command {
            name: command.into(),
            query: query.map(String::from),
        }
    }

    fn body(response: &Response) -> serde_json::Value {
        serde_json::from_slice(response.body.as_ref().unwrap()).unwrap()
    }

    #[tokio::test]
    async fn app_commands_answer_with_the_same_data_the_popup_reads() {
        let directory = tempfile::tempdir().unwrap();
        let app = mock_builder()
            .manage(UsageService::at(directory.path()))
            .manage(InsightsService::at(directory.path()))
            .build(mock_context(noop_assets()))
            .unwrap();
        let handle = app.handle().clone();

        let prices = respond(&handle, request("prices", None)).await.unwrap();
        assert_eq!(prices.status, 200);
        assert!(body(&prices).is_object() || body(&prices).is_array());

        let summary = respond(
            &handle,
            request("usage_summary", Some(r#"{"query":{"groupBy":"day"}}"#)),
        )
        .await
        .unwrap();
        assert_eq!(
            (summary.status, body(&summary)),
            (200, serde_json::json!([]))
        );

        let info = respond(&handle, request("usage_ledger_info", None))
            .await
            .unwrap();
        assert_eq!(info.status, 200);

        let contexts = respond(&handle, request("context_windows", None))
            .await
            .unwrap();
        assert_eq!(
            (contexts.status, body(&contexts)),
            (200, serde_json::json!([]))
        );

        let feed = respond(
            &handle,
            request("public_feed", Some(r#"{"name":"claudeResets"}"#)),
        )
        .await
        .unwrap();
        assert_eq!(feed.status, 200);

        for bad in [
            request("usage_summary", None),
            request("usage_summary", Some(r#"{"query":{"groupBy":"week"}}"#)),
            request("public_feed", Some(r#"{"name":"nope"}"#)),
            request("model_quality", Some("not json")),
        ] {
            let response = respond(&handle, bad).await.unwrap();
            assert_eq!(
                (response.status, body(&response)["error"].clone()),
                (400, serde_json::json!("bad_query"))
            );
        }

        assert!(
            respond(&handle, request("redeem_limit_reset", None))
                .await
                .is_none()
        );
    }
}
