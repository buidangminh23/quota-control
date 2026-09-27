//! Reading one value from an app's SQLite state (VS Code-style `state.vscdb`) without disturbing
//! the app that owns it: nothing is written beside its files and nothing is copied out of them.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension};

/// How a database can be read without leaving anything behind.
#[derive(Debug, PartialEq, Eq)]
enum Access {
    /// A normal read-only connection, which shares the owner's locks so a read never lands in the
    /// middle of one of its writes.
    Shared,
    /// Read as a file nobody changes: no locks and no write-ahead-log files. Only chosen when
    /// every page is in the main file.
    Immutable,
}

/// Run `query` (one row, first column, one text parameter) read-only against the database at
/// `path`.
///
/// SQLite opens a write-ahead-log database's log and shared-memory index even for reading, and
/// creates whichever is missing. So such a database is read through the owner's files only when
/// both exist, as an immutable file when the log holds nothing, and not at all while the log
/// still holds pages but its index is gone (the owner stopped in the middle of a write).
pub fn query_text(path: &Path, query: &str, parameter: &str) -> Option<String> {
    let connection = match access(path)? {
        Access::Shared => open(path).ok()?,
        Access::Immutable => open_immutable(path).ok()?,
    };
    read(&connection, query, parameter).ok().flatten()
}

/// The value of `key` in a VS Code-style `ItemTable`.
pub fn item(path: &Path, key: &str) -> Option<String> {
    query_text(path, "SELECT value FROM ItemTable WHERE key = ?1", key)
}

fn access(path: &Path) -> Option<Access> {
    let mut header = [0u8; 20];
    std::fs::File::open(path)
        .ok()?
        .read_exact(&mut header)
        .ok()?;
    if &header[..16] != b"SQLite format 3\0" {
        return None;
    }
    if header[19] != 2 {
        return Some(Access::Shared);
    }
    let log = std::fs::metadata(beside(path, "-wal")).ok();
    let index = beside(path, "-shm").is_file();
    match log {
        Some(_) if index => Some(Access::Shared),
        Some(log) if log.len() > 0 => None,
        _ => Some(Access::Immutable),
    }
}

/// `path` with `suffix` appended to its file name, as SQLite names the files it keeps beside a
/// database.
fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

fn open(path: &Path) -> rusqlite::Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(Duration::from_millis(750))?;
    Ok(connection)
}

fn open_immutable(path: &Path) -> rusqlite::Result<Connection> {
    let mut uri = url::Url::from_file_path(path)
        .map_err(|()| rusqlite::Error::InvalidPath(path.to_path_buf()))?;
    uri.set_query(Some("mode=ro&immutable=1"));
    Connection::open_with_flags(
        uri.as_str(),
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
}

fn read(connection: &Connection, query: &str, parameter: &str) -> rusqlite::Result<Option<String>> {
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

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);";

    fn files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn reads_an_item_table_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(&format!(
                "{TABLE} INSERT INTO ItemTable VALUES ('cursorAuth/accessToken', 'token-1');"
            ))
            .unwrap();
        drop(connection);
        assert_eq!(access(&path), Some(Access::Shared));
        assert_eq!(
            item(&path, "cursorAuth/accessToken").as_deref(),
            Some("token-1")
        );
        assert_eq!(item(&path, "missing"), None);
        assert_eq!(item(&dir.path().join("none.vscdb"), "x"), None);
        assert_eq!(files(dir.path()), ["state.vscdb"]);
    }

    #[test]
    fn a_closed_write_ahead_log_database_is_read_without_creating_its_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(&format!(
                "PRAGMA journal_mode=WAL; {TABLE} INSERT INTO ItemTable VALUES ('k', 'closed');"
            ))
            .unwrap();
        drop(connection);
        assert_eq!(files(dir.path()), ["state.vscdb"]);
        assert_eq!(access(&path), Some(Access::Immutable));
        assert_eq!(item(&path, "k").as_deref(), Some("closed"));
        assert_eq!(files(dir.path()), ["state.vscdb"]);
    }

    #[test]
    fn an_open_write_ahead_log_database_is_read_through_its_owners_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        let owner = Connection::open(&path).unwrap();
        owner
            .execute_batch(&format!(
                "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; {TABLE}
                 INSERT INTO ItemTable VALUES ('k', 'fresh');"
            ))
            .unwrap();
        let before = files(dir.path());
        assert_eq!(
            before,
            ["state.vscdb", "state.vscdb-shm", "state.vscdb-wal"]
        );
        assert_eq!(access(&path), Some(Access::Shared));
        assert_eq!(item(&path, "k").as_deref(), Some("fresh"));
        assert_eq!(files(dir.path()), before);
        drop(owner);
    }

    #[test]
    fn a_pending_log_without_its_index_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        let owner = Connection::open(&path).unwrap();
        owner
            .execute_batch(&format!(
                "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; {TABLE}
                 INSERT INTO ItemTable VALUES ('k', 'pending');"
            ))
            .unwrap();
        let copied = tempfile::tempdir().unwrap();
        let copy = copied.path().join("state.vscdb");
        std::fs::copy(&path, &copy).unwrap();
        std::fs::copy(beside(&path, "-wal"), beside(&copy, "-wal")).unwrap();
        drop(owner);
        assert_eq!(access(&copy), None);
        assert_eq!(item(&copy, "k"), None);
        assert_eq!(files(copied.path()), ["state.vscdb", "state.vscdb-wal"]);
    }

    #[test]
    fn files_that_are_not_databases_are_skipped_without_side_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        std::fs::write(&path, b"invalid sqlite").unwrap();
        assert_eq!(item(&path, "token"), None);
        std::fs::write(beside(&path, "-wal"), b"uncheckpointed").unwrap();
        assert_eq!(item(&path, "token"), None);
        assert_eq!(files(dir.path()), ["state.vscdb", "state.vscdb-wal"]);
    }
}
