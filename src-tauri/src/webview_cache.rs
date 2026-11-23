use std::path::Path;

const MARKER: &str = ".webview-cache-version";
const DISPOSABLE: [&str; 2] = ["Code Cache", "Cache"];

pub fn trim_after_update(profile_root: &Path, version: &str) {
    let marker = profile_root.join(MARKER);
    if std::fs::read_to_string(&marker).is_ok_and(|seen| seen.trim() == version) {
        return;
    }
    let browser = profile_root.join("EBWebView").join("Default");
    let mut freed = 0u64;
    let mut complete = true;
    for name in DISPOSABLE {
        let folder = browser.join(name);
        let Ok(metadata) = std::fs::symlink_metadata(&folder) else {
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        let size = folder_size(&folder);
        match std::fs::remove_dir_all(&folder) {
            Ok(()) => freed += size,
            Err(error) => {
                complete = false;
                tracing::info!("webview cache {name} kept for now: {error}");
            }
        }
    }
    if freed > 0 {
        tracing::info!(
            "webview cache trimmed after an update: {} MB",
            freed / 1_048_576
        );
    }
    if complete && std::fs::create_dir_all(profile_root).is_ok() {
        let _ = std::fs::write(&marker, version);
    }
}

fn folder_size(folder: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => folder_size(&entry.path()),
            Ok(kind) if kind.is_file() => entry.metadata().map(|meta| meta.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(root: &Path) {
        for name in ["Code Cache/js", "Cache/Cache_Data", "Local Storage"] {
            let folder = root.join("EBWebView").join("Default").join(name);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("entry"), vec![7u8; 4096]).unwrap();
        }
    }

    fn present(root: &Path, name: &str) -> bool {
        root.join("EBWebView").join("Default").join(name).exists()
    }

    #[test]
    fn a_new_version_drops_the_code_and_http_caches_only() {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path());
        trim_after_update(dir.path(), "0.3.57");
        assert!(!present(dir.path(), "Code Cache"));
        assert!(!present(dir.path(), "Cache"));
        assert!(present(dir.path(), "Local Storage"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join(MARKER)).unwrap(),
            "0.3.57"
        );
    }

    #[test]
    fn the_same_version_leaves_the_caches_alone() {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path());
        std::fs::write(dir.path().join(MARKER), "0.3.57").unwrap();
        trim_after_update(dir.path(), "0.3.57");
        assert!(present(dir.path(), "Code Cache"));
        assert!(present(dir.path(), "Cache"));
    }

    #[test]
    fn a_missing_profile_only_records_the_version() {
        let dir = tempfile::tempdir().unwrap();
        trim_after_update(dir.path(), "0.3.57");
        assert_eq!(
            std::fs::read_to_string(dir.path().join(MARKER)).unwrap(),
            "0.3.57"
        );
    }
}
