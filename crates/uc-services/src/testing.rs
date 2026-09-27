//! Test helpers: a scripted HTTP client and a fetch context around it.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uc_core::{HttpClient, HttpError, HttpRequest, HttpResponse, SharedHttpClient};

use crate::service::{FetchContext, Memo, Secret};

/// A scripted answer: status, lower-cased headers and body.
type Answer = (u16, Vec<(String, String)>, Vec<u8>);

struct Route {
    method: String,
    url: String,
    answers: VecDeque<Answer>,
}

#[derive(Default)]
struct Inner {
    routes: Mutex<Vec<Route>>,
    requests: Mutex<Vec<HttpRequest>>,
}

/// An HTTP client that answers from a script. A request matches the route with the same method
/// whose URL is the longest prefix of the request's URL; a route given several answers returns
/// them in order and then keeps repeating the last. Unmatched requests get a 599.
#[derive(Clone, Default)]
pub struct Scripted(Arc<Inner>);

impl Scripted {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on(self, method: &str, url: &str, status: u16, body: &str) -> Self {
        self.on_with_headers(method, url, status, &[], body)
    }

    pub fn on_with_headers(
        self,
        method: &str,
        url: &str,
        status: u16,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Self {
        let answer = (
            status,
            headers
                .iter()
                .map(|(name, value)| (name.to_ascii_lowercase(), value.to_string()))
                .collect(),
            body.as_bytes().to_vec(),
        );
        {
            let mut routes = self.0.routes.lock().unwrap();
            match routes
                .iter_mut()
                .find(|route| route.method == method && route.url == url)
            {
                Some(route) => route.answers.push_back(answer),
                None => routes.push(Route {
                    method: method.into(),
                    url: url.into(),
                    answers: VecDeque::from([answer]),
                }),
            }
        }
        self
    }

    /// Every request sent so far, in order.
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.0.requests.lock().unwrap().clone()
    }

    pub fn shared(&self) -> SharedHttpClient {
        Arc::new(self.clone())
    }
}

#[async_trait]
impl HttpClient for Scripted {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        self.0.requests.lock().unwrap().push(request.clone());
        let mut routes = self.0.routes.lock().unwrap();
        let route = routes
            .iter_mut()
            .filter(|route| route.method == request.method && request.url.starts_with(&route.url))
            .max_by_key(|route| route.url.len());
        let Some(route) = route else {
            return Ok(HttpResponse {
                status: 599,
                headers: Default::default(),
                body: format!("no scripted answer for {} {}", request.method, request.url)
                    .into_bytes(),
            });
        };
        let (status, headers, body) = if route.answers.len() > 1 {
            route.answers.pop_front().unwrap()
        } else {
            route.answers.front().cloned().unwrap()
        };
        Ok(HttpResponse {
            status,
            headers: headers.into_iter().collect(),
            body,
        })
    }
}

/// What a test fetch runs with; `context()` borrows it as a `FetchContext`.
pub struct Scope {
    secret: Secret,
    http: SharedHttpClient,
    memo: Memo,
    now: DateTime<Utc>,
}

impl Scope {
    pub fn context(&self) -> FetchContext<'_> {
        FetchContext {
            secret: &self.secret,
            http: &self.http,
            now: self.now,
            memo: &self.memo,
        }
    }
}

/// A fetch context at `now` with `secret`, answering from `http`.
pub fn context_at(http: &Scripted, secret: Value, now: DateTime<Utc>) -> Scope {
    Scope {
        secret: Secret::new(secret),
        http: http.shared(),
        memo: Memo::default(),
        now,
    }
}

/// A test scope for an account signed in to from Quota Control, whose token document the card owns.
pub fn owned_context_at(http: &Scripted, secret: Value, now: DateTime<Utc>) -> Scope {
    Scope {
        secret: Secret::owned(secret),
        http: http.shared(),
        memo: Memo::default(),
        now,
    }
}

impl Scope {
    /// The renewed token document a fetch handed its card to save.
    pub async fn renewed(&self) -> Option<Value> {
        self.memo.get(crate::service::RENEWED, self.now).await
    }
}

/// The header `name` of `request`, case-insensitively.
pub fn header<'a>(request: &'a HttpRequest, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}
