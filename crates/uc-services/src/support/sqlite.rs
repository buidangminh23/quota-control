//! Reading one value from an app's SQLite state (VS Code-style `state.vscdb`, Chromium cookie
//! stores) without disturbing the app that owns it.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension};

/// The largest database copied aside when the app holds it locked.
const MAX_COPY_BYTES: u64 = 512 * 1024 * 1024;

/// Run `query` (one row, first column, one text parameter) read-only against the database at
/// `path`. An app that holds the file locked is side-stepped by reading a private copy.
pub fn query_text(path: &Path, query: &str, parameter: &str) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    match query_in(path, query, parameter) {
        Ok(value) => value,
        Err(_) => query_copy(path, query, parameter),
    }
}

/// The value of `key` in a VS Code-style `ItemTable`.
pub fn item(path: &Path, key: &str) -> Option<String> {
    query_text(path, "SELECT value FROM ItemTable WHERE key = ?1", key)
}

fn open(path: &Path) -> rusqlite::Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(Duration::from_millis(750))?;
    Ok(connection)
}

fn query_in(path: &Path, query: &str, parameter: &str) -> rusqlite::Result<Option<String>> {
    let connection = open(path)?;
    let value: Option<rusqlite::types::Value> = connection
        .query_row(query, [parameter], |row| row.get(0))
        .optional()?;
    Ok(value.and_then(|value| match value {
        rusqlite::types::Value::Text(text) => Some(text),
        rusqlite::types::Value::Blob(bytes) => String::from_utf8(bytes).ok(),
        rusqlite::types::Value::Integer(number) => Some(number.to_string()),
        rusqlite::types::Value::Real(number) => Some(number.to_string()),
        rusqlite::types::Value::Null => None,
    }))
}

fn query_copy(path: &Path, query: &str, parameter: &str) -> Option<String> {
    if std::fs::metadata(path).ok()?.len() > MAX_COPY_BYTES {
        return None;
    }
    let dir = tempfile::tempdir().ok()?;
    let copy = dir.path().join("state.db");
    std::fs::copy(path, &copy).ok()?;
    for suffix in ["-wal", "-shm"] {
        let side = path.with_file_name(format!("{}{suffix}", path.file_name()?.to_string_lossy()));
        if side.is_file() {
            let _ = std::fs::copy(&side, dir.path().join(format!("state.db{suffix}")));
        }
    }
    query_in(&copy, query, parameter).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_item_table_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);
                 INSERT INTO ItemTable VALUES ('cursorAuth/accessToken', 'token-1');",
            )
            .unwrap();
        drop(connection);
        assert_eq!(
            item(&path, "cursorAuth/accessToken").as_deref(),
            Some("token-1")
        );
        assert_eq!(item(&path, "missing"), None);
        assert_eq!(item(&dir.path().join("none.vscdb"), "x"), None);
    }
}
