use std::sync::Arc;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use tauri::{AppHandle, Manager};
use uc_api::{AppRequest, Response};
use uc_logscan::ledger::UsageQuery;
use uc_logscan::quality::QualityQuery;

use crate::account_commands::Accounts;
use crate::insights_commands::InsightsService;
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

pub fn handler(app: AppHandle) -> uc_api::server::AppHandler {
    Arc::new(move |request| {
        let app = app.clone();
        Box::pin(async move { respond(&app, request).await })
    })
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
