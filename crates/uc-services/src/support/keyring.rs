//! Reading a secret another app keeps in the operating system's credential store: Windows
//! Credential Manager, the macOS login keychain and the Linux Secret Service. Read-only: nothing
//! here writes or deletes an entry.

/// A secret saved with go-keyring's layout (the Go library the GitHub CLI and Antigravity use):
/// the Windows generic credential `service:user`, the macOS generic password with that service and
/// account, the Linux Secret Service item with attributes `service` and `username`.
pub fn go_keyring(service: &str, user: &str) -> Option<String> {
    let value = platform::go_keyring(service, user)?;
    Some(decode_go_keyring(value.trim()))
}

/// go-keyring on macOS may store a value as `go-keyring-base64:<base64>`.
fn decode_go_keyring(value: &str) -> String {
    use base64::Engine;
    if let Some(encoded) = value.strip_prefix("go-keyring-base64:")
        && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded)
        && let Ok(text) = String::from_utf8(bytes)
    {
        return text;
    }
    if let Some(encoded) = value.strip_prefix("go-keyring-encoded:")
        && let Ok(bytes) = hex_decode(encoded)
        && let Ok(text) = String::from_utf8(bytes)
    {
        return text;
    }
    value.to_string()
}

fn hex_decode(text: &str) -> Result<Vec<u8>, ()> {
    if !text.len().is_multiple_of(2) {
        return Err(());
    }
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).map_err(|_| ()))
        .collect()
}

/// Every Windows generic credential whose target starts with `prefix`, as `(target, user,
/// secret)`. Empty on other platforms.
pub fn windows_prefixed(prefix: &str) -> Vec<(String, String, String)> {
    platform::windows_prefixed(prefix)
}

/// A credential blob as text: UTF-16LE when it looks like it (every second byte zero for ASCII),
/// UTF-8 otherwise.
pub fn blob_text(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let looks_wide = bytes.len() >= 2
        && bytes.len().is_multiple_of(2)
        && bytes
            .chunks(2)
            .take(8)
            .all(|pair| pair[1] == 0 && pair[0] != 0);
    if looks_wide {
        let units: Vec<u16> = bytes
            .chunks(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        return String::from_utf16(&units)
            .ok()
            .map(|text| text.trim_end_matches('\0').to_string());
    }
    String::from_utf8(bytes.to_vec()).ok()
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::Security::Credentials::{
        CRED_TYPE_GENERIC, CREDENTIALW, CredEnumerateW, CredFree, CredReadW,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe fn read_wide(pointer: *const u16) -> String {
        if pointer.is_null() {
            return String::new();
        }
        let mut length = 0;
        unsafe {
            while *pointer.add(length) != 0 {
                length += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length))
        }
    }

    unsafe fn secret_of(credential: &CREDENTIALW) -> Option<String> {
        if credential.CredentialBlob.is_null() || credential.CredentialBlobSize == 0 {
            return None;
        }
        let bytes = unsafe {
            std::slice::from_raw_parts(
                credential.CredentialBlob,
                credential.CredentialBlobSize as usize,
            )
        };
        super::blob_text(bytes)
    }

    pub fn go_keyring(service: &str, user: &str) -> Option<String> {
        let target = wide(&format!("{service}:{user}"));
        let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
        let found = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) };
        if found == 0 || credential.is_null() {
            return None;
        }
        let secret = unsafe { secret_of(&*credential) };
        unsafe { CredFree(credential.cast()) };
        secret
    }

    pub fn windows_prefixed(prefix: &str) -> Vec<(String, String, String)> {
        let filter = wide(&format!("{prefix}*"));
        let mut count = 0u32;
        let mut list: *mut *mut CREDENTIALW = std::ptr::null_mut();
        let found = unsafe { CredEnumerateW(filter.as_ptr(), 0, &mut count, &mut list) };
        if found == 0 || list.is_null() {
            return Vec::new();
        }
        let mut entries = Vec::new();
        for index in 0..count as usize {
            let credential = unsafe { *list.add(index) };
            if credential.is_null() {
                continue;
            }
            let credential = unsafe { &*credential };
            if credential.Type != CRED_TYPE_GENERIC {
                continue;
            }
            let target = unsafe { read_wide(credential.TargetName) };
            let user = unsafe { read_wide(credential.UserName) };
            if let Some(secret) = unsafe { secret_of(credential) } {
                entries.push((target, user, secret));
            }
        }
        unsafe { CredFree(list.cast()) };
        entries
    }
}

#[cfg(target_os = "macos")]
mod platform {
    pub fn go_keyring(service: &str, user: &str) -> Option<String> {
        let output = std::process::Command::new("/usr/bin/security")
            .args(["find-generic-password", "-s", service, "-a", user, "-w"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .filter(|value| !value.is_empty())
    }

    pub fn windows_prefixed(_prefix: &str) -> Vec<(String, String, String)> {
        Vec::new()
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    pub fn go_keyring(service: &str, user: &str) -> Option<String> {
        let output = std::process::Command::new("secret-tool")
            .args(["lookup", "service", service, "username", user])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .filter(|value| !value.is_empty())
    }

    pub fn windows_prefixed(_prefix: &str) -> Vec<(String, String, String)> {
        Vec::new()
    }
}

#[cfg(not(any(windows, unix)))]
mod platform {
    pub fn go_keyring(_service: &str, _user: &str) -> Option<String> {
        None
    }

    pub fn windows_prefixed(_prefix: &str) -> Vec<(String, String, String)> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blobs_decode_as_utf16_or_utf8() {
        let wide: Vec<u8> = "gho_token"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(blob_text(&wide).as_deref(), Some("gho_token"));
        assert_eq!(blob_text(b"{\"a\":1}").as_deref(), Some("{\"a\":1}"));
        assert_eq!(blob_text(b""), None);
    }

    #[test]
    fn go_keyring_encodings_are_undone() {
        assert_eq!(decode_go_keyring("go-keyring-base64:aGVsbG8="), "hello");
        assert_eq!(decode_go_keyring("go-keyring-encoded:68656c6c6f"), "hello");
        assert_eq!(decode_go_keyring("plain"), "plain");
    }
}
