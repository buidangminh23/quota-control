//! IBM Bob: the Bobcoins the key's user spent in each IBM Bob team, against the team budgets.
//!
//! Nothing is read from disk on Windows, macOS or Linux: the key comes from the
//! `BOBSHELL_API_KEY` environment variable or from a key saved in Quota Control. A key that is a
//! JWT is sent as `Authorization: Bearer`, any other key as `Authorization: Apikey`. A refresh
//! reads the account's instances and teams from
//! `GET https://api.us-east.bob.ibm.com/admin/v1/profile`, whose answer is remembered for 12 hours,
//! then sends `GET /admin/v1/teams/{team}/users/{user}` to each instance's regional host with
//! `x-instance-id` and `x-team-id` headers, for three teams at most. A regional host that is not a
//! plain HTTPS host under `bob.ibm.com` fails the refresh instead. The card shows a total row
//! first, then one row per team, and warns when teams past the third were left out. When every
//! team has a budget, the total is a meter against their sum that renews at the earliest
//! `refresh_at`; otherwise it shows the Bobcoins alone. The plan is the instances' plan names. The
//! requests follow CodexBar's `IBMBobUsageFetcher.swift`.

use async_trait::async_trait;
use chrono::Duration;
use serde_json::Value;
use uc_core::{HttpRequest, Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, jwt, lines, value};

pub(crate) struct IbmBob;

const BASE: &str = "https://api.us-east.bob.ibm.com";

/// The API address of an instance's `region_domain`, US East when it names none. It must be a
/// plain HTTPS host under `bob.ibm.com`, without a port, credentials, path, query or fragment.
fn region(raw: Option<&str>) -> Result<url::Url, SimpleProviderError> {
    let raw = raw.unwrap_or(BASE);
    let text = if raw.contains("://") {
        raw.to_owned()
    } else if raw.starts_with("api.") {
        format!("https://{raw}")
    } else {
        format!("https://api.{raw}")
    };
    let address = url::Url::parse(&text)
        .map_err(|_| http::invalid("IBM Bob returned an untrusted regional API address."))?;
    if address.scheme() != "https"
        || address.port().is_some()
        || !address.username().is_empty()
        || address.password().is_some()
        || address.query().is_some()
        || address.fragment().is_some()
        || address.path() != "/"
        || !address
            .host_str()
            .is_some_and(|host| host == "bob.ibm.com" || host.ends_with(".bob.ibm.com"))
    {
        return Err(http::invalid(
            "IBM Bob returned an untrusted regional API address.",
        ));
    }
    Ok(address)
}

#[async_trait]
impl Service for IbmBob {
    fn id(&self) -> &'static str {
        "ibmbob"
    }

    fn name(&self) -> &'static str {
        "IBM Bob"
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["BOBSHELL_API_KEY"],
            url: "https://bob.ibm.com",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![WidgetDescriptor::bounded_count(
            format!("{}.credits", provider.id),
            provider,
            "Bobcoins",
            None,
            0.0,
            "Bobcoins",
            Some(lines::MONTH_MS),
        )]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context
            .secret
            .key()
            .ok_or_else(|| http::invalid("The IBM Bob API key is missing."))?;
        let auth = if jwt::claims(key).is_some() {
            format!("Bearer {key}")
        } else {
            format!("Apikey {key}")
        };
        let profile = if let Some(saved) = context.memo.get("ibmbob-profile", context.now).await {
            saved
        } else {
            let fetched = http::json(
                context.http,
                HttpRequest::get(format!("{BASE}/admin/v1/profile")).header("Authorization", &auth),
                "IBM Bob",
            )
            .await?;
            if !fetched.get("instances").is_some_and(Value::is_array) {
                return Err(http::decoding("IBM Bob"));
            }
            context
                .memo
                .put(
                    "ibmbob-profile",
                    fetched.clone(),
                    Some(context.now + Duration::hours(12)),
                )
                .await;
            fetched
        };
        let instances = profile
            .get("instances")
            .and_then(Value::as_array)
            .ok_or_else(|| http::decoding("IBM Bob"))?;
        let mut rows = Vec::new();
        let mut used = 0.0;
        let mut total = Some(0.0);
        let mut resets = Vec::new();
        let mut plans = std::collections::BTreeSet::new();
        let mut count = 0;
        let mut truncated = false;
        for instance in instances {
            let (Some(user_id), Some(instance_id)) = (
                value::text(instance, "/user_id"),
                value::text(instance, "/instance_id"),
            ) else {
                continue;
            };
            let Some(teams) = instance.get("teams").and_then(Value::as_array) else {
                continue;
            };
            let base = region(value::text(instance, "/region_domain"))?;
            for team in teams {
                let Some(team_id) = value::text(team, "/id") else {
                    continue;
                };
                if count >= 3 {
                    truncated = true;
                    continue;
                }
                count += 1;
                let mut url = base.clone();
                url.path_segments_mut()
                    .map_err(|_| http::decoding("IBM Bob"))?
                    .extend(["admin", "v1", "teams", team_id, "users", user_id]);
                let member = http::json(
                    context.http,
                    HttpRequest::get(url.as_str())
                        .header("Authorization", &auth)
                        .header("x-instance-id", instance_id)
                        .header("x-team-id", team_id),
                    "IBM Bob",
                )
                .await?;
                let amount = value::number(&member, "/usage")
                    .ok_or_else(|| http::decoding("IBM Bob"))?
                    .max(0.0);
                let limit = value::number(&member, "/budget_limit")
                    .or_else(|| value::number(team, "/budget_limit"))
                    .filter(|limit| *limit >= 0.0);
                let reset = value::time(instance, "/refresh_at");
                if let Some(at) = reset {
                    resets.push(at);
                }
                if let Some(plan) = value::text(instance, "/plan_name") {
                    plans.insert(plan.to_owned());
                }
                used += amount;
                total = total.zip(limit).map(|(sum, budget)| sum + budget);
                let label = format!(
                    "{} · {}",
                    value::text(instance, "/instance_name")
                        .or_else(|| value::text(instance, "/name"))
                        .unwrap_or(instance_id),
                    value::text(team, "/name").unwrap_or(team_id)
                );
                rows.push(if let Some(limit) = limit {
                    lines::count(
                        &label,
                        amount,
                        limit,
                        "Bobcoins",
                        reset,
                        Some(lines::MONTH_MS),
                    )
                } else {
                    lines::count_value(&label, amount, "Bobcoins")
                });
            }
        }
        if rows.is_empty() {
            return Err(http::not_available(
                "IBM Bob returned no subscription teams for this key.",
            ));
        }
        rows.insert(
            0,
            if let Some(total) = total {
                lines::count(
                    "Bobcoins",
                    used,
                    total,
                    "Bobcoins",
                    resets.into_iter().min(),
                    Some(lines::MONTH_MS),
                )
            } else {
                lines::count_value("Bobcoins", used, "Bobcoins")
            },
        );
        Ok(Reading::new(
            (!plans.is_empty()).then(|| plans.into_iter().collect::<Vec<_>>().join(", ")),
            rows,
        )
        .with_warning(
            truncated
                .then(|| "Only the first three IBM Bob teams are included in this reading.".into()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at, header};
    use chrono::Utc;
    use serde_json::json;
    use uc_core::ErrorCategory;

    const PROFILE: &str = r#"{"instances":[{"instance_id":"i","instance_name":"Org","user_id":"u","plan_name":"Pro","refresh_at":1790812800,"teams":[{"id":"t","name":"Team","budget_limit":100}]}]}"#;

    #[tokio::test]
    async fn reads_the_user_budget_of_each_team_with_an_apikey_header_and_remembers_the_profile() {
        let http = Scripted::new()
            .on("GET", &format!("{BASE}/admin/v1/profile"), 200, PROFILE)
            .on(
                "GET",
                &format!("{BASE}/admin/v1/teams/t/users/u"),
                200,
                r#"{"usage":25,"budget_limit":200}"#,
            );
        let scope = context_at(&http, json!({"apiKey": "key"}), Utc::now());
        let reading = IbmBob.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        assert_eq!(
            reading.lines,
            vec![
                lines::count(
                    "Bobcoins",
                    25.0,
                    200.0,
                    "Bobcoins",
                    value::as_time(&json!(1790812800)),
                    Some(lines::MONTH_MS)
                ),
                lines::count(
                    "Org · Team",
                    25.0,
                    200.0,
                    "Bobcoins",
                    value::as_time(&json!(1790812800)),
                    Some(lines::MONTH_MS)
                )
            ]
        );
        assert!(
            scope
                .context()
                .memo
                .get("ibmbob-profile", scope.context().now)
                .await
                .is_some()
        );
        assert_eq!(
            header(&http.requests()[1], "Authorization"),
            Some("Apikey key")
        );
        assert_eq!(header(&http.requests()[1], "x-team-id"), Some("t"));
    }

    #[test]
    fn a_region_must_be_a_plain_https_host_under_bob_ibm_com() {
        for raw in [
            "https://evil.com",
            "https://bob.ibm.com.evil.com",
            "https://u:p@bob.ibm.com",
            "https://bob.ibm.com/path",
            "http://bob.ibm.com",
            "https://bob.ibm.com:8443",
        ] {
            assert!(region(Some(raw)).is_err());
        }
        assert_eq!(
            region(Some("eu-de.bob.ibm.com")).unwrap().host_str(),
            Some("api.eu-de.bob.ibm.com")
        );
    }

    #[tokio::test]
    async fn reads_at_most_three_teams_and_warns_about_the_rest() {
        let profile = json!({
            "instances": [{
                "instance_id": "i",
                "user_id": "u",
                "teams": [{"id": "a"}, {"id": "b"}, {"id": "c"}, {"id": "d"}]
            }]
        })
        .to_string();
        let mut http =
            Scripted::new().on("GET", &format!("{BASE}/admin/v1/profile"), 200, &profile);
        for team in ["a", "b", "c"] {
            http = http.on(
                "GET",
                &format!("{BASE}/admin/v1/teams/{team}/users/u"),
                200,
                r#"{"usage":10}"#,
            );
        }
        let scope = context_at(&http, json!({"apiKey": "key"}), Utc::now());
        let reading = IbmBob.fetch(&scope.context()).await.unwrap();
        assert_eq!(http.requests().len(), 4);
        assert_eq!(
            reading.lines[0],
            lines::count_value("Bobcoins", 30.0, "Bobcoins")
        );
        assert!(reading.warning.is_some());
    }

    #[tokio::test]
    async fn a_refused_limited_or_unreadable_profile_keeps_its_error_category() {
        for (status, category) in [
            (401, ErrorCategory::AuthExpired),
            (429, ErrorCategory::RateLimited),
            (200, ErrorCategory::Decoding),
        ] {
            let http = Scripted::new().on("GET", &format!("{BASE}/admin/v1/profile"), status, "{}");
            let scope = context_at(&http, json!({"apiKey": "test"}), Utc::now());
            assert_eq!(
                IbmBob.fetch(&scope.context()).await.unwrap_err().category,
                category
            );
        }
    }
}
