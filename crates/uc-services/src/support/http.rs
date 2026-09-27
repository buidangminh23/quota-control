//! Sending a service request and turning its failures into card errors.

use serde_json::Value;
use uc_core::{ErrorCategory, HttpRequest, HttpResponse, SharedHttpClient, SimpleProviderError};

/// Send `request` and return the response, whatever its status.
pub async fn send(
    http: &SharedHttpClient,
    request: HttpRequest,
    service: &str,
) -> Result<HttpResponse, SimpleProviderError> {
    http.send(request).await.map_err(|_| {
        SimpleProviderError::new(
            ErrorCategory::Network,
            format!("Cannot connect to {service}. Check the connection and try again."),
        )
    })
}

/// The error a failed response stands for: 401 and 403 mean the login or key no longer works,
/// 429 means too many requests, anything else is reported by its status.
pub fn status_error(response: &HttpResponse, service: &str) -> SimpleProviderError {
    match response.status {
        401 | 403 => SimpleProviderError::new(
            ErrorCategory::AuthExpired,
            format!("{service} refused the saved login or key. Sign in again or replace the key."),
        ),
        429 => SimpleProviderError::new(
            ErrorCategory::RateLimited,
            format!("{service} is rate limiting usage requests. Waiting before retrying."),
        ),
        status => SimpleProviderError::new(
            ErrorCategory::http(status),
            format!("{service} answered with HTTP {status}."),
        ),
    }
}

/// Send `request` and parse a successful JSON answer; failures become card errors.
pub async fn json(
    http: &SharedHttpClient,
    request: HttpRequest,
    service: &str,
) -> Result<Value, SimpleProviderError> {
    let response = send(http, request, service).await?;
    if !response.is_success() {
        return Err(status_error(&response, service));
    }
    parse(&response, service)
}

/// A response body as JSON.
pub fn parse(response: &HttpResponse, service: &str) -> Result<Value, SimpleProviderError> {
    response.json::<Value>().map_err(|_| decoding(service))
}

pub fn decoding(service: &str) -> SimpleProviderError {
    SimpleProviderError::new(
        ErrorCategory::Decoding,
        format!("{service} returned usage data this version cannot read."),
    )
}

/// A login or key that exists but lacks what the request needs.
pub fn invalid(message: impl Into<String>) -> SimpleProviderError {
    SimpleProviderError::new(ErrorCategory::AuthInvalid, message)
}

/// Usage that this account or plan does not have.
pub fn not_available(message: impl Into<String>) -> SimpleProviderError {
    SimpleProviderError::new(ErrorCategory::NotAvailable, message)
}

pub fn expired(message: impl Into<String>) -> SimpleProviderError {
    SimpleProviderError::new(ErrorCategory::AuthExpired, message)
}
