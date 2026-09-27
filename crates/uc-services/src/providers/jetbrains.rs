//! JetBrains AI: the AI Assistant quota a JetBrains IDE last saved on this computer, shown as the
//! share of its Monthly quota used. No request is sent and nothing is written: the reading comes
//! from the IDE's own local snapshot.
//!
//! Each IDE keeps its settings in a folder named after the product and its version (such as
//! `IntelliJIdea2026.2`) under `JetBrains` in the roaming application data: `%APPDATA%` on
//! Windows, `~/Library/Application Support` on macOS and the XDG config folder on Linux. The quota
//! comes from that folder's `options/AIAssistantQuotaManager2.xml` (a regular file of at most
//! 1 MiB): the `quotaInfo` option (plan type, current and maximum) and the `nextRefill` option of
//! its `AIAssistantQuotaManager2` component. Each product gives one login, read from its newest
//! version.

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde_json::{Value, json};
use uc_core::{Provider, SimpleProviderError, WidgetDescriptor};

use crate::service::{Connection, FetchContext, Login, Reading, Roots, Secret, Service};
use crate::support::{http, lines, value};

pub(crate) struct JetBrains;

#[async_trait]
impl Service for JetBrains {
    fn id(&self) -> &'static str {
        "jetbrains"
    }

    fn name(&self) -> &'static str {
        "JetBrains AI"
    }

    fn connection(&self) -> Connection {
        Connection::login("JetBrains IDE")
    }

    fn discover(&self, roots: &Roots) -> Vec<Login> {
        let mut latest: BTreeMap<String, (Vec<u32>, Login)> = BTreeMap::new();
        let Ok(entries) = std::fs::read_dir(roots.app_data.join("JetBrains")) else {
            return vec![];
        };
        for entry in entries.flatten().take(1024) {
            if !entry.file_type().ok().is_some_and(|kind| kind.is_dir()) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(index) = name.find(|character: char| character.is_ascii_digit()) else {
                continue;
            };
            let family = &name[..index];
            let version: Vec<u32> = name[index..]
                .split('.')
                .map(|part| part.parse().unwrap_or(0))
                .collect();
            let path = entry.path().join("options/AIAssistantQuotaManager2.xml");
            if std::fs::symlink_metadata(&path)
                .ok()
                .is_none_or(|metadata| !metadata.is_file() || metadata.len() > 1_048_576)
            {
                continue;
            }
            let Some(data) = std::fs::read_to_string(&path)
                .ok()
                .and_then(|xml| parse(&xml))
            else {
                continue;
            };
            let login = Login::new(family, "JetBrains IDE", &path, Secret::new(data))
                .with_label(Some(family.into()));
            if latest
                .get(family)
                .is_none_or(|(newest, _)| version > *newest)
            {
                latest.insert(family.into(), (version, login));
            }
        }
        latest.into_values().map(|(_, login)| login).collect()
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        vec![
            WidgetDescriptor::percent(
                format!("{}.monthly", provider.id),
                provider,
                "Monthly",
                None,
                None,
            )
            .exporting_progress("monthly", "percent"),
        ]
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let snapshot = context.secret.value();
        let used = value::number(snapshot, "/quota/current")
            .ok_or_else(|| http::decoding("JetBrains AI"))?;
        let maximum = value::number(snapshot, "/quota/maximum")
            .filter(|maximum| *maximum > 0.0)
            .ok_or_else(|| {
                http::not_available("JetBrains AI has no positive quota in its local snapshot.")
            })?;
        Ok(Reading::new(
            value::text(snapshot, "/quota/type").and_then(lines::plan_name),
            vec![lines::percent(
                "Monthly",
                used / maximum * 100.0,
                value::time(snapshot, "/refill/next"),
                None,
            )],
        ))
    }
}

/// The value of the attribute `name` in the XML start tag `tag`, in single or double quotes.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = tag;
    while let Some((_, tail)) = rest.split_once(name) {
        rest = tail;
        let tail = tail.trim_start();
        let Some(tail) = tail.strip_prefix('=') else {
            continue;
        };
        let tail = tail.trim_start();
        let quote = tail.chars().next()?;
        if quote != '"' && quote != '\'' {
            continue;
        }
        return tail[1..].split_once(quote).map(|(quoted, _)| quoted);
    }
    None
}

/// The `quotaInfo` and `nextRefill` options of the `AIAssistantQuotaManager2` component, each
/// option value decoded from its XML entities and read as JSON; `None` without that component or
/// without a readable `quotaInfo`.
fn parse(xml: &str) -> Option<Value> {
    let component = xml.split("<component").find(|fragment| {
        fragment
            .split_once('>')
            .is_some_and(|(tag, _)| attribute(tag, "name") == Some("AIAssistantQuotaManager2"))
    })?;
    let body = component.split_once('>')?.1.split("</component>").next()?;
    let mut quota = None;
    let mut refill = None;
    for tag in body
        .split("<option")
        .skip(1)
        .filter_map(|fragment| fragment.split_once('>').map(|(tag, _)| tag))
    {
        let name = attribute(tag, "name");
        let Some(encoded) = attribute(tag, "value") else {
            continue;
        };
        let decoded = encoded
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&#10;", "\n")
            .replace("&amp;", "&");
        let parsed = serde_json::from_str::<Value>(&decoded).ok();
        match name {
            Some("quotaInfo") => quota = parsed,
            Some("nextRefill") => refill = parsed,
            _ => {}
        }
    }
    Some(json!({"quota":quota?,"refill":refill}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Scripted, context_at};
    use chrono::Utc;

    const XML: &str = r#"<application><component name="AIAssistantQuotaManager2"><option name="quotaInfo" value="{&quot;type&quot;:&quot;ai_pro&quot;,&quot;current&quot;:&quot;25&quot;,&quot;maximum&quot;:&quot;100&quot;}" /><option name="nextRefill" value="{&quot;next&quot;:&quot;2026-10-01T00:00:00Z&quot;}" /></component></application>"#;

    #[tokio::test]
    async fn the_saved_quota_reads_as_a_monthly_share_without_any_request() {
        let http = Scripted::new();
        let scope = context_at(&http, parse(XML).unwrap(), Utc::now());
        let reading = JetBrains.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Ai Pro"));
        assert_eq!(
            reading.lines,
            vec![lines::percent(
                "Monthly",
                25.0,
                value::as_time(&json!("2026-10-01T00:00:00Z")),
                None
            )]
        );
        assert!(http.requests().is_empty());
    }

    #[test]
    fn discovery_selects_latest_per_family() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path());
        assert!(JetBrains.discover(&roots).is_empty());
        for name in ["IntelliJIdea2026.2", "IntelliJIdea2026.10", "PyCharm2026.1"] {
            let options = roots.app_data.join("JetBrains").join(name).join("options");
            std::fs::create_dir_all(&options).unwrap();
            std::fs::write(options.join("AIAssistantQuotaManager2.xml"), XML).unwrap();
        }
        let logins = JetBrains.discover(&roots);
        assert_eq!(logins.len(), 2);
        assert!(logins[0].location.to_string_lossy().contains("2026.10"));
        assert_eq!(logins[1].identity, "PyCharm");
    }

    #[test]
    fn malformed_xml_or_quota_json_gives_no_snapshot() {
        assert!(parse("<broken>").is_none());
        assert!(parse(&XML.replace("&quot;current&quot;", "invalid")).is_none());
    }
}
