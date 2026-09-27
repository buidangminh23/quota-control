//! Base URLs a user gives for a service they host themselves or reach through their own gateway
//! (LiteLLM, Bifrost, sub2api): HTTPS anywhere, plain HTTP only where the policy allows it, never
//! with credentials in the URL.

use std::net::IpAddr;

use uc_core::SimpleProviderError;
use url::{Host, Url};

use crate::support::http;

/// Where plain HTTP is allowed besides HTTPS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// HTTPS only.
    Https,
    /// HTTPS, or HTTP to this computer.
    HttpsOrLoopbackHttp,
    /// HTTPS, or HTTP to this computer or a private network (10/8, 172.16/12, 192.168/16,
    /// link-local, IPv6 unique-local, `localhost` and `.local`, `.lan`, `.internal`, `.home.arpa`).
    HttpsOrPrivateNetworkHttp,
}

/// The base URL in `value`, else `default`, without a trailing slash. An unusable URL is an
/// [`http::invalid`] error naming `service`, so the card says what to fix.
pub fn base_url(
    value: Option<&str>,
    default: Option<&str>,
    policy: Policy,
    service: &str,
) -> Result<String, SimpleProviderError> {
    let raw = value
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .or(default)
        .ok_or_else(|| http::invalid(format!("Enter the {service} server address.")))?;
    let url = Url::parse(raw)
        .map_err(|_| http::invalid(format!("The {service} server address is not a valid URL.")))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(http::invalid(format!(
            "Remove the user name and password from the {service} server address."
        )));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(http::invalid(format!(
            "The {service} server address cannot have a query or fragment."
        )));
    }
    let allowed = match url.scheme() {
        "https" => url.host().is_some(),
        "http" => match policy {
            Policy::Https => false,
            Policy::HttpsOrLoopbackHttp => url.host().is_some_and(|host| is_loopback(&host)),
            Policy::HttpsOrPrivateNetworkHttp => url
                .host()
                .is_some_and(|host| is_loopback(&host) || is_private(&host)),
        },
        _ => false,
    };
    if !allowed {
        return Err(http::invalid(match policy {
            Policy::Https => format!("The {service} server address must start with https://."),
            Policy::HttpsOrLoopbackHttp => format!(
                "The {service} server address must use https://, or http:// to this computer."
            ),
            Policy::HttpsOrPrivateNetworkHttp => format!(
                "The {service} server address must use https://, or http:// to a private network."
            ),
        }));
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn is_loopback(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(address) => address.is_loopback(),
        Host::Ipv6(address) => address.is_loopback(),
    }
}

fn is_private(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(name) => {
            let name = name.to_ascii_lowercase();
            [".local", ".lan", ".internal", ".home.arpa"]
                .iter()
                .any(|suffix| name.ends_with(suffix))
        }
        Host::Ipv4(address) => address.is_private() || address.is_link_local(),
        Host::Ipv6(address) => {
            let first = address.segments()[0];
            (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || address
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| is_private(&Host::Ipv4(mapped)) || mapped.is_loopback())
        }
    }
}

/// Whether `address` is on this computer or a private network, for callers holding a bare IP.
pub fn is_private_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            is_loopback(&Host::Ipv4(address)) || is_private(&Host::Ipv4(address))
        }
        IpAddr::V6(address) => {
            is_loopback(&Host::Ipv6(address)) || is_private(&Host::Ipv6(address))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(value: &str, policy: Policy) -> Option<String> {
        base_url(Some(value), None, policy, "LiteLLM").ok()
    }

    #[test]
    fn https_is_always_allowed_and_the_trailing_slash_dropped() {
        assert_eq!(
            ok("https://llm.example.com/", Policy::Https).as_deref(),
            Some("https://llm.example.com")
        );
        assert_eq!(
            ok(" https://llm.example.com/v1/ ", Policy::Https).as_deref(),
            Some("https://llm.example.com/v1")
        );
    }

    #[test]
    fn plain_http_follows_the_policy() {
        assert!(ok("http://llm.example.com", Policy::HttpsOrPrivateNetworkHttp).is_none());
        assert!(ok("http://127.0.0.1:4000", Policy::Https).is_none());
        assert!(ok("http://127.0.0.1:4000", Policy::HttpsOrLoopbackHttp).is_some());
        assert!(ok("http://localhost:4000", Policy::HttpsOrLoopbackHttp).is_some());
        assert!(ok("http://192.168.1.20:4000", Policy::HttpsOrLoopbackHttp).is_none());
        for private in [
            "http://192.168.1.20:4000",
            "http://10.0.0.5",
            "http://172.20.1.1",
            "http://[fd00::1]:8080",
            "http://gateway.lan",
            "http://nas.local:4000",
        ] {
            assert!(
                ok(private, Policy::HttpsOrPrivateNetworkHttp).is_some(),
                "{private}"
            );
        }
        assert!(ok("http://172.32.0.1", Policy::HttpsOrPrivateNetworkHttp).is_none());
    }

    #[test]
    fn credentials_queries_and_other_schemes_are_refused() {
        assert!(ok("https://user:pass@llm.example.com", Policy::Https).is_none());
        assert!(ok("https://llm.example.com/?key=1", Policy::Https).is_none());
        assert!(ok("ftp://llm.example.com", Policy::Https).is_none());
        assert!(ok("not a url", Policy::Https).is_none());
    }

    #[test]
    fn the_default_stands_in_for_a_blank_value() {
        assert_eq!(
            base_url(
                Some("  "),
                Some("https://api.example.com"),
                Policy::Https,
                "X"
            )
            .unwrap(),
            "https://api.example.com"
        );
        let error = base_url(None, None, Policy::Https, "LiteLLM").unwrap_err();
        assert_eq!(error.category, uc_core::ErrorCategory::AuthInvalid);
    }
}
