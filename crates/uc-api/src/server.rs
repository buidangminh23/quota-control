//! Loopback-only HTTP/1.1 listener for the read-only API on `127.0.0.1:6736` (upstream
//! `LocalUsageServer.swift`). When the port is taken the API is simply off for the session. At most
//! 16 requests are served at once; beyond that a connection gets `503 {"error":"server_busy"}`.

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

use crate::router::{Response, respond};
use crate::state::ApiState;

pub const PORT: u16 = 6736;
const MAX_CONNECTIONS: usize = 16;
const HEAD_LIMIT: usize = 8192;
/// A client that never finishes its request head gives its slot back after this long.
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);

/// Bind the API's loopback port. Fails when another process holds it.
pub async fn bind() -> std::io::Result<TcpListener> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, PORT)).await
}

/// Answer requests on `listener` forever; `state` is captured once per request.
pub async fn serve<F>(listener: TcpListener, state: F)
where
    F: Fn() -> ApiState + Send + Sync + 'static,
{
    let state = Arc::new(state);
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let mut stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(error) => {
                tracing::warn!(target: "local_api", "accept failed: {error}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            tokio::spawn(async move { send(&mut stream, &Response::busy()).await });
            continue;
        };
        let state = state.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let Ok(Some(head)) = tokio::time::timeout(HEAD_TIMEOUT, read_head(&mut stream)).await
            else {
                return;
            };
            let (method, path) = parse_request_line(&head);
            tracing::debug!(target: "local_api", "{method} {path}");
            send(&mut stream, &respond(&method, &path, &state())).await;
        });
    }
}

/// Read up to the end of the request head. GET and OPTIONS bodies are irrelevant, so the head is
/// all the router needs. `None` when the client stops early or sends an oversized head.
async fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            return Some(String::from_utf8_lossy(&buffer[..end]).into_owned());
        }
        if buffer.len() >= HEAD_LIMIT {
            return None;
        }
    }
}

/// The request line's method and path. A head without one routes to a normal 404, never a panic.
pub(crate) fn parse_request_line(head: &str) -> (String, String) {
    let line = head.split("\r\n").next().unwrap_or_default();
    let mut parts = line.split(' ').filter(|part| !part.is_empty());
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or("/").to_string();
    (method, path)
}

async fn send(stream: &mut TcpStream, response: &Response) {
    let reason = match response.status {
        204 => "No Content",
        404 => "Not Found",
        405 => "Method Not Allowed",
        503 => "Service Unavailable",
        _ => "OK",
    };
    let mut bytes = format!(
        "HTTP/1.1 {} {reason}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nConnection: close\r\n",
        response.status
    )
    .into_bytes();
    match &response.body {
        Some(body) => {
            bytes.extend_from_slice(
                format!(
                    "Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            );
            bytes.extend_from_slice(body);
        }
        None => bytes.extend_from_slice(b"Content-Length: 0\r\n\r\n"),
    }
    if stream.write_all(&bytes).await.is_err() || stream.shutdown().await.is_err() {
        return;
    }
    let _ = tokio::time::timeout(LINGER, drain(stream)).await;
}

/// How long a closing connection keeps reading what the client still sends. Closing a socket with
/// unread input resets it, and a reset can discard the reply before the client reads it.
const LINGER: Duration = Duration::from_secs(1);

async fn drain(stream: &mut TcpStream) {
    let mut chunk = [0_u8; 1024];
    while matches!(stream.read(&mut chunk).await, Ok(read) if read > 0) {}
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};
    use std::net::SocketAddr;

    use chrono::Utc;

    use super::*;

    fn empty_state() -> ApiState {
        ApiState {
            enabled_ordered_ids: vec![],
            known_ids: BTreeSet::from(["codex".to_string()]),
            snapshots: HashMap::new(),
            limit_descriptors: HashMap::new(),
            errors: HashMap::new(),
            generated_at: Utc::now(),
            refresh_interval: Duration::from_secs(300),
        }
    }

    async fn start() -> SocketAddr {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(serve(listener, empty_state));
        address
    }

    async fn exchange(address: SocketAddr, request: &[u8]) -> String {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream.write_all(request).await.unwrap();
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply).await.unwrap();
        String::from_utf8(reply).unwrap()
    }

    #[test]
    fn request_lines_parse_defensively() {
        assert_eq!(
            parse_request_line("GET /v1/limits HTTP/1.1\r\nHost: x"),
            ("GET".into(), "/v1/limits".into())
        );
        assert_eq!(parse_request_line(""), ("".into(), "/".into()));
        assert_eq!(parse_request_line("\r\n"), ("".into(), "/".into()));
        assert_eq!(parse_request_line("GET"), ("GET".into(), "/".into()));
    }

    #[tokio::test]
    async fn serves_json_with_cors_and_closes() {
        let address = start().await;
        let reply = exchange(
            address,
            b"GET /v1/limits HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
        )
        .await;
        let (head, body) = reply.split_once("\r\n\r\n").unwrap();
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
        assert!(head.contains("Access-Control-Allow-Origin: *"));
        assert!(head.contains("Connection: close"));
        assert!(head.contains(&format!("Content-Length: {}", body.len())));
        let json: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(json["schema"], crate::SCHEMA);

        let preflight = exchange(address, b"OPTIONS /v1/limits HTTP/1.1\r\n\r\n").await;
        assert!(preflight.starts_with("HTTP/1.1 204 No Content\r\n"));
        assert!(preflight.ends_with("Content-Length: 0\r\n\r\n"));
        let garbage = exchange(address, b"\xff\xfe\r\n\r\n").await;
        assert!(
            garbage.starts_with("HTTP/1.1 404 Not Found\r\n"),
            "{garbage}"
        );
    }

    #[tokio::test]
    async fn oversized_heads_are_dropped_without_a_reply() {
        let address = start().await;
        let request = [b'a'; HEAD_LIMIT + 16];
        let mut stream = TcpStream::connect(address).await.unwrap();
        let _ = stream.write_all(&request).await;
        let mut reply = Vec::new();
        let _ = stream.read_to_end(&mut reply).await;
        assert!(reply.is_empty());
    }

    #[tokio::test]
    async fn the_seventeenth_concurrent_connection_is_told_to_back_off() {
        let address = start().await;
        let mut idle = Vec::new();
        for _ in 0..MAX_CONNECTIONS {
            idle.push(TcpStream::connect(address).await.unwrap());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        let busy = exchange(address, b"GET /v1/limits HTTP/1.1\r\n\r\n").await;
        assert!(
            busy.starts_with("HTTP/1.1 503 Service Unavailable\r\n"),
            "{busy}"
        );
        assert!(busy.ends_with(r#"{"error":"server_busy"}"#));
        drop(idle);
        tokio::time::sleep(Duration::from_millis(200)).await;
        let ok = exchange(address, b"GET /v1/usage HTTP/1.1\r\n\r\n").await;
        assert!(ok.starts_with("HTTP/1.1 200 OK\r\n"), "{ok}");
    }
}
