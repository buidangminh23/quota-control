//! Where Usage Control keeps its files on each platform.
//!
//! | Purpose | Windows | Linux |
//! |---|---|---|
//! | settings, layout | `%APPDATA%\UsageControl` | `$XDG_CONFIG_HOME/usage-control` |
//! | caches (snapshots, log scans, pricing) | `%LOCALAPPDATA%\UsageControl\Cache` | `$XDG_CACHE_HOME/usage-control` |
//! | log file | `%LOCALAPPDATA%\UsageControl\Logs` | `$XDG_STATE_HOME/usage-control` |
//! | user config (proxy) | `~/.usage-control/config.json` | `~/.usage-control/config.json` |
//!
//! `USAGE_CONTROL_HOME` redirects everything under one directory (tests, portable installs).

use std::path::PathBuf;

pub const APP_DIR_WINDOWS: &str = "UsageControl";
pub const APP_DIR_UNIX: &str = "usage-control";
pub const HOME_OVERRIDE_ENV: &str = "USAGE_CONTROL_HOME";

fn override_root() -> Option<PathBuf> {
    std::env::var_os(HOME_OVERRIDE_ENV).filter(|value| !value.is_empty()).map(PathBuf::from)
}

/// The user's home directory. Providers resolve their credential files relative to it.
pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_dir() -> PathBuf {
    if let Some(root) = override_root() {
        return root.join("config");
    }
    if cfg!(windows) {
        dirs::config_dir().unwrap_or_else(home_dir).join(APP_DIR_WINDOWS)
    } else {
        dirs::config_dir().unwrap_or_else(|| home_dir().join(".config")).join(APP_DIR_UNIX)
    }
}

pub fn cache_dir() -> PathBuf {
    if let Some(root) = override_root() {
        return root.join("cache");
    }
    if cfg!(windows) {
        dirs::data_local_dir().unwrap_or_else(home_dir).join(APP_DIR_WINDOWS).join("Cache")
    } else {
        dirs::cache_dir().unwrap_or_else(|| home_dir().join(".cache")).join(APP_DIR_UNIX)
    }
}

pub fn log_dir() -> PathBuf {
    if let Some(root) = override_root() {
        return root.join("logs");
    }
    if cfg!(windows) {
        dirs::data_local_dir().unwrap_or_else(home_dir).join(APP_DIR_WINDOWS).join("Logs")
    } else {
        dirs::state_dir().unwrap_or_else(|| home_dir().join(".local").join("state")).join(APP_DIR_UNIX)
    }
}

pub fn log_file() -> PathBuf {
    log_dir().join("UsageControl.log")
}

/// `~/.usage-control/config.json`: the hand-edited user config (proxy), like upstream's
/// `~/.openusage/config.json`.
pub fn user_config_file() -> PathBuf {
    if let Some(root) = override_root() {
        return root.join("user").join("config.json");
    }
    home_dir().join(".usage-control").join("config.json")
}

/// Expand a leading `~` or `~/` to the home directory.
pub fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        return home_dir();
    }
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        return home_dir().join(rest);
    }
    PathBuf::from(path)
}

/// Read an environment variable, treating empty values as unset.
pub fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|value| !value.is_empty()).map(|value| expand_tilde(&value.to_string_lossy()))
}

/// Write `bytes` to `path` atomically: temp file in the same directory, flushed, then renamed.
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path.parent().ok_or_else(|| std::io::Error::other("path has no parent directory"))?;
    std::fs::create_dir_all(parent)?;
    let file_name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let temp = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    match std::fs::rename(&temp, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_file(&temp);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_expands_to_home() {
        assert_eq!(expand_tilde("~/.claude"), home_dir().join(".claude"));
        assert_eq!(expand_tilde("/etc/hosts"), PathBuf::from("/etc/hosts"));
    }

    #[test]
    fn atomic_write_replaces_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("state.json");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
    }
}
