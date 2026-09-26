//! Claude Code on macOS keeps its login in the login keychain instead of `.credentials.json`
//! (upstream `SecurityKeychainAccessor`). The item is read through `/usr/bin/security`, the tool
//! Claude Code stores it with, so the item's access list already trusts the reader and macOS shows
//! no prompt. Only the password query prints the secret, and it goes straight to the JSON parser.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};

const SECURITY: &str = "/usr/bin/security";
/// `security` exits with `errSecItemNotFound` (44) when no item matches.
const ITEM_NOT_FOUND: i32 = 44;
const TIMEOUT: Duration = Duration::from_secs(5);
/// After the keychain refuses a password (locked, denied, or `security` still waiting on its unlock
/// dialog when the timeout hit), it is left alone this long, so a locked keychain costs one dialog
/// and one timeout per window instead of one on every refresh.
const REFUSAL_BACKOFF: Duration = Duration::from_secs(5 * 60);

static REFUSED_AT: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
const MAX_OUTPUT: u64 = 1_048_576;
const CLAUDE_SERVICE: &str = "Claude Code-credentials";

/// One generic-password item: its service name and, when known, the account it is filed under.
/// Claude Code files its login under the current user; older versions left the account empty.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeychainItem {
    pub service: String,
    pub account: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeychainError {
    /// `security` could not run, timed out, or the keychain refused (locked or denied).
    Unavailable,
}

impl KeychainItem {
    /// Claude Code's item under the current user, honouring `CLAUDE_CONFIG_DIR`.
    pub fn claude() -> Self {
        Self {
            service: claude_service(std::env::var("CLAUDE_CONFIG_DIR").ok().as_deref()),
            account: current_user(),
        }
    }

    fn query(&self, reveal: bool) -> Vec<String> {
        let mut arguments = vec!["find-generic-password".to_owned()];
        if let Some(account) = &self.account {
            arguments.extend(["-a".to_owned(), account.clone()]);
        }
        arguments.extend(["-s".to_owned(), self.service.clone()]);
        if reveal {
            arguments.push("-w".to_owned());
        }
        arguments
    }
}

/// Claude Code's service name: `Claude Code-credentials`, followed by the first eight hex digits of
/// SHA-256 of `CLAUDE_CONFIG_DIR` when that variable is set.
pub fn claude_service(config_dir: Option<&str>) -> String {
    match config_dir.map(str::trim).filter(|value| !value.is_empty()) {
        Some(directory) => {
            let digest = Sha256::digest(directory.as_bytes());
            let suffix: String = digest
                .iter()
                .take(4)
                .map(|byte| format!("{byte:02x}"))
                .collect();
            format!("{CLAUDE_SERVICE}-{suffix}")
        }
        None => CLAUDE_SERVICE.to_owned(),
    }
}

fn current_user() -> Option<String> {
    std::env::var("USER")
        .ok()
        .map(|user| user.trim().to_owned())
        .filter(|user| !user.is_empty())
        .or_else(|| {
            uc_core::paths::home_dir()
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
}

/// The item `security` finds for Claude Code: filed under the current user first, then under any
/// account for logins older versions saved. `Ok(None)` when neither exists.
pub fn find_claude() -> Result<Option<KeychainItem>, KeychainError> {
    let current = KeychainItem::claude();
    let legacy = KeychainItem {
        account: None,
        ..current.clone()
    };
    for item in [current, legacy] {
        if attributes(&item)?.is_some() {
            return Ok(Some(item));
        }
    }
    Ok(None)
}

/// The item's password, or `None` when the item no longer exists. Refused reads back off for
/// [`REFUSAL_BACKOFF`] before the keychain is asked again.
pub fn password(item: &KeychainItem) -> Result<Option<String>, KeychainError> {
    let mut refused = REFUSED_AT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if backing_off(*refused, Instant::now()) {
        return Err(KeychainError::Unavailable);
    }
    let result = run(&item.query(true));
    *refused = result.is_err().then(Instant::now);
    drop(refused);
    let Some(output) = result? else {
        return Ok(None);
    };
    let text = output.trim();
    Ok((!text.is_empty()).then(|| text.to_owned()))
}

/// When the item last changed (its `mdat` attribute), which moves whenever Claude Code renews or
/// replaces the login.
pub fn modified_at(item: &KeychainItem) -> Option<DateTime<Utc>> {
    attributes(item)
        .ok()
        .flatten()
        .as_deref()
        .and_then(parse_modified_at)
}

fn backing_off(refused_at: Option<Instant>, now: Instant) -> bool {
    refused_at.is_some_and(|at| now.saturating_duration_since(at) < REFUSAL_BACKOFF)
}

fn attributes(item: &KeychainItem) -> Result<Option<String>, KeychainError> {
    run(&item.query(false))
}

/// Run `security`; `Ok(None)` means the item does not exist.
fn run(arguments: &[String]) -> Result<Option<String>, KeychainError> {
    let mut child = Command::new(SECURITY)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| KeychainError::Unavailable)?;
    let mut stdout = child.stdout.take().ok_or(KeychainError::Unavailable)?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = (&mut stdout).take(MAX_OUTPUT).read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let bytes = reader.join().unwrap_or_default();
    match status.and_then(|status| status.code()) {
        Some(0) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| KeychainError::Unavailable),
        Some(ITEM_NOT_FOUND) => Ok(None),
        _ => Err(KeychainError::Unavailable),
    }
}

/// The login document a keychain password holds: JSON, or JSON that `security` printed as hex
/// because it was not plain text.
pub fn decode_document(secret: &str) -> Option<Value> {
    let text = secret.trim();
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return Some(value);
    }
    let hex = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    if hex.is_empty() || hex.len() % 2 != 0 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).ok())
        .collect();
    serde_json::from_slice(&bytes?).ok()
}

/// `mdat` from `security find-generic-password` output, e.g.
/// `"mdat"<timedate>=0x3230…00  "20260922093616Z\000"`.
pub fn parse_modified_at(attributes: &str) -> Option<DateTime<Utc>> {
    let line = attributes
        .lines()
        .find(|line| line.trim_start().starts_with("\"mdat\"<timedate>="))?;
    let quoted = line.split('"').nth(3)?;
    let digits = quoted.get(..14)?;
    NaiveDateTime::parse_from_str(digits, "%Y%m%d%H%M%S")
        .ok()
        .map(|time| time.and_utc())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_is_left_alone_for_the_backoff_window() {
        let now = Instant::now();
        assert!(!backing_off(None, now));
        assert!(backing_off(Some(now), now + Duration::from_secs(60)));
        assert!(!backing_off(Some(now), now + REFUSAL_BACKOFF));
    }

    #[test]
    fn claude_service_matches_claude_code_names() {
        assert_eq!(claude_service(None), "Claude Code-credentials");
        assert_eq!(claude_service(Some("  ")), "Claude Code-credentials");
        let digest = Sha256::digest(b"/Users/a/.claude-work");
        let expected: String = digest
            .iter()
            .take(4)
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(
            claude_service(Some("/Users/a/.claude-work")),
            format!("Claude Code-credentials-{expected}")
        );
    }

    #[test]
    fn documents_decode_from_json_or_hex() {
        let json = r#"{"claudeAiOauth":{"accessToken":"a"}}"#;
        assert_eq!(
            decode_document(json).unwrap()["claudeAiOauth"]["accessToken"],
            "a"
        );
        let hex: String = json.bytes().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(
            decode_document(&hex).unwrap(),
            decode_document(json).unwrap()
        );
        assert_eq!(
            decode_document(&format!("0x{hex}\n")).unwrap(),
            decode_document(json).unwrap()
        );
        assert!(decode_document("abc").is_none());
        assert!(decode_document("").is_none());
    }

    #[test]
    fn modification_time_comes_from_the_mdat_attribute() {
        let output = concat!(
            "keychain: \"/Users/a/Library/Keychains/login.keychain-db\"\n",
            "attributes:\n",
            "    \"cdat\"<timedate>=0x32303236303731363034313731375A00  \"20260716041717Z\\000\"\n",
            "    \"mdat\"<timedate>=0x32303236303932323039333631365A00  \"20260922093616Z\\000\"\n",
        );
        assert_eq!(
            parse_modified_at(output).unwrap().to_rfc3339(),
            "2026-09-22T09:36:16+00:00"
        );
        assert!(parse_modified_at("attributes:\n").is_none());
    }

    #[test]
    fn queries_name_the_account_only_when_known() {
        let item = KeychainItem {
            service: "svc".into(),
            account: Some("me".into()),
        };
        assert_eq!(
            item.query(true),
            ["find-generic-password", "-a", "me", "-s", "svc", "-w"]
        );
        let legacy = KeychainItem {
            account: None,
            ..item
        };
        assert_eq!(legacy.query(false), ["find-generic-password", "-s", "svc"]);
    }
}
