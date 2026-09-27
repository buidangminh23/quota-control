//! Where desktop apps keep their state, and reading what VS Code-style apps (VS Code, Cursor,
//! Windsurf, Kiro, Trae) store for their extensions: the `ItemTable` of `state.vscdb` and the
//! extension secret storage, which Electron encrypts with a key only this user can unwrap.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::service::Roots;
use crate::support::sqlite;

/// VS Code-family app folders under the roaming app data folder, most common first.
pub const VSCODE_APPS: [&str; 5] = ["Code", "Code - Insiders", "VSCodium", "Cursor", "Windsurf"];

/// The `User/globalStorage/state.vscdb` of the VS Code-style app in folder `app` ("Cursor").
pub fn state_db(roots: &Roots, app: &str) -> PathBuf {
    roots
        .app_data
        .join(app)
        .join("User")
        .join("globalStorage")
        .join("state.vscdb")
}

/// The `globalStorage/<extension>` folder of `app`, where an extension keeps its own files.
pub fn extension_storage(roots: &Roots, app: &str, extension: &str) -> PathBuf {
    roots
        .app_data
        .join(app)
        .join("User")
        .join("globalStorage")
        .join(extension)
}

/// The `ItemTable` value of `key` in `app`'s state database.
pub fn item(roots: &Roots, app: &str, key: &str) -> Option<String> {
    sqlite::item(&state_db(roots, app), key)
}

/// A secret an extension stored with VS Code's `SecretStorage` (`context.secrets`) in `app`.
/// Windows only: elsewhere the key that unlocks it sits in the login keychain behind a prompt, so
/// this returns `None` there.
pub fn secret(roots: &Roots, app: &str, extension: &str, key: &str) -> Option<String> {
    let item_key = format!("secret://{{\"extensionId\":\"{extension}\",\"key\":\"{key}\"}}");
    let stored = item(roots, app, &item_key)?;
    let bytes = buffer_bytes(&stored)?;
    let plain = unseal(&roots.app_data.join(app).join("Local State"), &bytes)?;
    String::from_utf8(plain).ok()
}

/// Bytes an Electron app sealed with `safeStorage`, opened with the key in its `Local State`.
/// Windows only, like [`secret`].
pub fn unseal(local_state: &Path, sealed: &[u8]) -> Option<Vec<u8>> {
    open_sealed(&master_key(local_state)?, sealed)
}

/// The bytes of a Node `Buffer` serialized as JSON (`{"type":"Buffer","data":[...]}`).
fn buffer_bytes(stored: &str) -> Option<Vec<u8>> {
    let parsed: Value = serde_json::from_str(stored).ok()?;
    let data = parsed.get("data")?.as_array()?;
    data.iter()
        .map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
        .collect()
}

#[cfg(windows)]
fn master_key(local_state: &Path) -> Option<Vec<u8>> {
    use crate::support::value;
    use base64::Engine;
    let state = value::read_json(local_state, 4 * 1024 * 1024)?;
    let encoded = value::text(&state, "/os_crypt/encrypted_key")?;
    let wrapped = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let wrapped = wrapped.strip_prefix(b"DPAPI")?;
    dpapi_unprotect(wrapped)
}

#[cfg(not(windows))]
fn master_key(_local_state: &Path) -> Option<Vec<u8>> {
    None
}

#[cfg(windows)]
fn dpapi_unprotect(bytes: &[u8]) -> Option<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptUnprotectData,
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let success = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if success == 0 || output.pbData.is_null() {
        return None;
    }
    let key = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        std::ptr::write_bytes(output.pbData, 0, output.cbData as usize);
        LocalFree(output.pbData.cast());
    }
    Some(key)
}

/// Electron `safeStorage` on Windows: `v10` + 12-byte nonce + AES-256-GCM ciphertext and tag.
pub(crate) fn open_sealed(master: &[u8], bytes: &[u8]) -> Option<Vec<u8>> {
    use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
    let body = bytes.strip_prefix(b"v10")?;
    if body.len() < 12 + 16 {
        return None;
    }
    let (nonce, sealed) = body.split_at(12);
    let key = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, master).ok()?);
    let mut buffer = sealed.to_vec();
    let plain = key
        .open_in_place(
            Nonce::try_assume_unique_for_key(nonce).ok()?,
            Aad::empty(),
            &mut buffer,
        )
        .ok()?;
    Some(plain.to_vec())
}

/// The first of `paths` that is a file.
pub fn first_file(paths: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    paths.into_iter().find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};

    #[test]
    fn electron_safe_storage_blobs_decrypt_with_the_master_key() {
        let master = [7u8; 32];
        let nonce = [3u8; 12];
        let key = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &master).unwrap());
        let mut sealed = b"[{\"accessToken\":\"gho_x\"}]".to_vec();
        key.seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::empty(),
            &mut sealed,
        )
        .unwrap();
        let mut blob = b"v10".to_vec();
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&sealed);
        assert_eq!(
            open_sealed(&master, &blob).as_deref(),
            Some(b"[{\"accessToken\":\"gho_x\"}]".as_slice())
        );
        assert_eq!(open_sealed(&[8u8; 32], &blob), None);
        assert_eq!(open_sealed(&master, b"v11abc"), None);
    }

    #[test]
    fn node_buffers_parse_from_json() {
        assert_eq!(
            buffer_bytes(r#"{"type":"Buffer","data":[118,49,48]}"#),
            Some(b"v10".to_vec())
        );
        assert_eq!(buffer_bytes(r#"{"type":"Buffer","data":[300]}"#), None);
        assert_eq!(buffer_bytes("nope"), None);
    }

    #[test]
    fn state_databases_sit_under_app_data() {
        let roots = Roots::under(Path::new("/home/me"));
        let path = state_db(&roots, "Cursor");
        assert!(
            path.ends_with(
                Path::new("Cursor")
                    .join("User")
                    .join("globalStorage")
                    .join("state.vscdb")
            )
        );
        assert!(path.starts_with(&roots.app_data));
    }
}
