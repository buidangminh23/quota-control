use std::collections::BTreeMap;

use serde_json::Value;
use uc_core::http::{HttpClient, HttpRequest};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ReadinessError {
    Incomplete,
    Network,
}

const ROOT: &str = "https://github.com/buidangminh23/quota-control/releases/download";
const TARGETS: [(&str, &str); 7] = [
    ("windows-x86_64-nsis", "x64-setup.exe"),
    ("windows-x86_64", "x64-setup.exe"),
    ("linux-x86_64-deb", "amd64.deb"),
    ("linux-x86_64-appimage", "amd64.AppImage"),
    ("linux-x86_64", "amd64.AppImage"),
    ("darwin-aarch64-app", "aarch64.app.tar.gz"),
    ("darwin-aarch64", "aarch64.app.tar.gz"),
];

fn packages(manifest: &Value, version: &str) -> Result<BTreeMap<String, String>, ReadinessError> {
    if manifest["version"].as_str() != Some(version) || semver::Version::parse(version).is_err() {
        return Err(ReadinessError::Incomplete);
    }
    let mut packages = BTreeMap::new();
    for (target, suffix) in TARGETS {
        let entry = &manifest["platforms"][target];
        let signature = entry["signature"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or(ReadinessError::Incomplete)?;
        let suffix = if suffix == "aarch64.app.tar.gz"
            && entry["url"]
                .as_str()
                .is_some_and(|url| url.ends_with("_universal.app.tar.gz"))
        {
            "universal.app.tar.gz"
        } else {
            suffix
        };
        let name = format!("Quota-Control_{version}_{suffix}");
        let expected = format!("{ROOT}/v{version}/{name}");
        if entry["url"].as_str() != Some(expected.as_str()) {
            return Err(ReadinessError::Incomplete);
        }
        if let Some(previous) = packages.insert(name, signature.to_owned())
            && previous != signature
        {
            return Err(ReadinessError::Incomplete);
        }
    }
    for (primary, alias) in [
        ("windows-x86_64-nsis", "windows-x86_64"),
        ("linux-x86_64-appimage", "linux-x86_64"),
        ("darwin-aarch64-app", "darwin-aarch64"),
    ] {
        if manifest["platforms"][primary] != manifest["platforms"][alias] {
            return Err(ReadinessError::Incomplete);
        }
    }
    Ok(packages)
}

fn response_status(status: u16) -> Result<(), ReadinessError> {
    match status {
        200 => Ok(()),
        408 | 429 | 500..=599 => Err(ReadinessError::Network),
        _ => Err(ReadinessError::Incomplete),
    }
}

pub(super) async fn verify(
    http: &dyn HttpClient,
    manifest: &Value,
    version: &str,
) -> Result<(), ReadinessError> {
    let packages = packages(manifest, version)?;
    let base = format!("{ROOT}/v{version}");
    let sums = http
        .send(HttpRequest::get(format!("{base}/SHA256SUMS")).max_response_bytes(32 * 1024))
        .await
        .map_err(|_| ReadinessError::Network)?;
    response_status(sums.status)?;
    let mut checksums = BTreeMap::new();
    for line in sums.text().lines() {
        let Some((hash, name)) = line.split_once("  ") else {
            return Err(ReadinessError::Incomplete);
        };
        if hash.len() != 64
            || !hash.bytes().all(|b| b.is_ascii_hexdigit())
            || checksums.insert(name.to_owned(), hash.to_owned()).is_some()
        {
            return Err(ReadinessError::Incomplete);
        }
    }
    let mac_arch = if packages
        .keys()
        .any(|name| name.ends_with("_universal.app.tar.gz"))
    {
        "universal"
    } else {
        "aarch64"
    };
    let dmg = format!("Quota-Control_{version}_{mac_arch}.dmg");
    for name in packages.keys().chain(std::iter::once(&dmg)) {
        if !checksums.contains_key(name) {
            return Err(ReadinessError::Incomplete);
        }
        let response = http
            .send(HttpRequest::new("HEAD", format!("{base}/{name}")))
            .await
            .map_err(|_| ReadinessError::Network)?;
        response_status(response.status)?;
        if response
            .header("content-length")
            .and_then(|s| s.parse::<u64>().ok())
            .is_none_or(|size| size == 0)
        {
            return Err(ReadinessError::Incomplete);
        }
        if let Some(signature) = packages.get(name) {
            let response = http
                .send(HttpRequest::get(format!("{base}/{name}.sig")).max_response_bytes(16 * 1024))
                .await
                .map_err(|_| ReadinessError::Network)?;
            response_status(response.status)?;
            if response.text().trim() != signature.trim() {
                return Err(ReadinessError::Incomplete);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use uc_core::http::{HttpError, HttpResponse};

    fn manifest() -> Value {
        let mut value = serde_json::json!({"version":"0.3.10", "platforms":{}});
        for (target, suffix) in TARGETS {
            value["platforms"][target] = serde_json::json!({"signature":"signed-package", "url":format!("{ROOT}/v0.3.10/Quota-Control_0.3.10_{suffix}")});
        }
        value
    }

    struct Release {
        status: u16,
        missing: Option<&'static str>,
        signature: &'static str,
    }
    #[async_trait::async_trait]
    impl HttpClient for Release {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
            assert!(request.method == "HEAD" || request.max_response_bytes.is_some());
            let mut response = HttpResponse {
                status: self.status,
                headers: HashMap::from([("content-length".into(), "123".into())]),
                body: Vec::new(),
            };
            if self
                .missing
                .is_some_and(|suffix| request.url.ends_with(suffix))
            {
                response.status = 404;
            }
            if request.url.ends_with("SHA256SUMS") {
                let mut names = packages(&manifest(), "0.3.10")
                    .unwrap()
                    .into_keys()
                    .collect::<Vec<_>>();
                names.push("Quota-Control_0.3.10_aarch64.dmg".into());
                if self.missing == Some("checksum") {
                    names.pop();
                }
                response.body = names
                    .iter()
                    .map(|name| format!("{}  {name}\n", "a".repeat(64)))
                    .collect::<String>()
                    .into_bytes();
            } else if request.url.ends_with(".sig") {
                response.body = self.signature.as_bytes().to_vec();
            }
            Ok(response)
        }
    }

    #[tokio::test]
    #[ignore = "read-only network probe against published GitHub assets"]
    async fn published_release_passes_the_real_http_client() {
        let http = uc_core::http::ReqwestHttpClient::shared();
        let response = http
            .send(
                HttpRequest::get(format!("{ROOT}/v0.3.10/latest.json"))
                    .max_response_bytes(64 * 1024),
            )
            .await
            .unwrap();
        assert_eq!(response.status, 200);
        let manifest: Value = response.json().unwrap();
        assert_eq!(verify(http.as_ref(), &manifest, "0.3.10").await, Ok(()));
    }

    #[test]
    fn requires_every_target_and_matching_aliases() {
        let valid = manifest();
        let mut universal = valid.clone();
        for target in ["darwin-aarch64", "darwin-aarch64-app"] {
            universal["platforms"][target]["url"] =
                format!("{ROOT}/v0.3.10/Quota-Control_0.3.10_universal.app.tar.gz").into();
        }
        assert_eq!(packages(&universal, "0.3.10").unwrap().len(), 4);
        assert_eq!(packages(&valid, "0.3.10").unwrap().len(), 4);
        for (target, _) in TARGETS {
            let mut incomplete = valid.clone();
            incomplete["platforms"]
                .as_object_mut()
                .unwrap()
                .remove(target);
            assert_eq!(
                packages(&incomplete, "0.3.10"),
                Err(ReadinessError::Incomplete)
            );
        }
        let mut invalid = valid.clone();
        invalid["platforms"]["windows-x86_64"]["signature"] = "different".into();
        assert_eq!(
            packages(&invalid, "0.3.10"),
            Err(ReadinessError::Incomplete)
        );
        assert_eq!(packages(&valid, "0.3.11"), Err(ReadinessError::Incomplete));
        invalid = valid.clone();
        invalid["platforms"]["linux-x86_64-deb"]["url"] = "https://example.org/package.deb".into();
        assert_eq!(
            packages(&invalid, "0.3.10"),
            Err(ReadinessError::Incomplete)
        );
    }

    #[tokio::test]
    async fn complete_release_passes_but_missing_packages_and_signatures_do_not() {
        let release = Release {
            status: 200,
            missing: None,
            signature: "signed-package",
        };
        assert_eq!(verify(&release, &manifest(), "0.3.10").await, Ok(()));
        for missing in [".dmg", ".deb", ".exe.sig", "SHA256SUMS", "checksum"] {
            let release = Release {
                missing: Some(missing),
                ..release
            };
            assert_eq!(
                verify(&release, &manifest(), "0.3.10").await,
                Err(ReadinessError::Incomplete)
            );
        }
        let release = Release {
            signature: "mismatched",
            ..release
        };
        assert_eq!(
            verify(&release, &manifest(), "0.3.10").await,
            Err(ReadinessError::Incomplete)
        );
    }

    #[tokio::test]
    async fn network_failure_is_distinct_from_an_unfinished_release() {
        let release = Release {
            status: 503,
            missing: None,
            signature: "signed-package",
        };
        assert_eq!(
            verify(&release, &manifest(), "0.3.10").await,
            Err(ReadinessError::Network)
        );
        assert_eq!(response_status(429), Err(ReadinessError::Network));
        assert_eq!(response_status(404), Err(ReadinessError::Incomplete));
    }
}
