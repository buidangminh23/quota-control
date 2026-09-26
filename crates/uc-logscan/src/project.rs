use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

pub(crate) fn resolve(cwd: &str, repository: Option<&str>) -> String {
    let path = Path::new(cwd);
    if !cwd.is_empty() && path.is_dir() {
        for ancestor in path.ancestors() {
            if ancestor.file_name().is_some_and(|name| name == "worktrees")
                && let Some(parent) = ancestor.parent()
                && parent.file_name().is_some_and(|name| name == ".claude")
                && let Some(main) = parent.parent()
            {
                let marker = main.join(".git");
                if marker.is_dir() {
                    return folder_name(main);
                }
                if marker.is_file() {
                    return linked_repository(&marker)
                        .as_deref()
                        .map(folder_name)
                        .unwrap_or_else(|| folder_name(main));
                }
            }
        }
        for ancestor in path.ancestors() {
            let marker = ancestor.join(".git");
            if marker.is_dir() {
                return folder_name(ancestor);
            }
            if marker.is_file() {
                if let Some(main) = linked_repository(&marker) {
                    return folder_name(&main);
                }
                return folder_name(ancestor);
            }
        }
    }
    if let Some(name) = repository.and_then(repository_name) {
        return name;
    }
    let normalized = cwd.replace('\\', "/");
    let mut parts = Vec::new();
    for part in normalized.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    if let Some(index) = parts
        .windows(2)
        .position(|pair| pair == [".claude", "worktrees"])
        && index > 0
    {
        return safe_name(parts[index - 1]).unwrap_or_default();
    }
    parts
        .last()
        .and_then(|part| safe_name(part))
        .unwrap_or_default()
}

fn linked_repository(marker: &Path) -> Option<PathBuf> {
    let content = small_text(marker)?;
    let gitdir = content.trim().strip_prefix("gitdir:")?.trim();
    let gitdir = absolute_from(marker.parent()?, gitdir);
    let common = small_text(&gitdir.join("commondir"))?;
    let common = absolute_from(&gitdir, common.trim()).canonicalize().ok()?;
    (common.file_name()? == ".git")
        .then(|| common.parent().map(Path::to_path_buf))
        .flatten()
}

fn absolute_from(base: &Path, raw: &str) -> PathBuf {
    let path = Path::new(raw);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn small_text(path: &Path) -> Option<String> {
    let mut text = String::new();
    File::open(path)
        .ok()?
        .take(4097)
        .read_to_string(&mut text)
        .ok()?;
    (text.len() <= 4096).then_some(text)
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .and_then(safe_name)
        .unwrap_or_default()
}

fn repository_name(repository: &str) -> Option<String> {
    let repository = repository
        .trim()
        .split(['?', '#'])
        .next()?
        .trim_end_matches('/');
    let name = repository.rsplit(['/', ':']).next()?;
    safe_name(name.strip_suffix(".git").unwrap_or(name))
}

fn safe_name(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty()
        && name.len() <= 256
        && name != "."
        && name != ".."
        && !name
            .chars()
            .any(|value| value.is_control() || matches!(value, '/' | '\\' | ':')))
    .then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_root_and_nested_paths_use_root_name() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("sample-project");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let nested = root.join("src").join("deep");
        std::fs::create_dir_all(&nested).unwrap();
        for path in [&root, &nested] {
            assert_eq!(resolve(&path.to_string_lossy(), None), "sample-project");
        }
    }

    #[test]
    fn linked_worktree_uses_common_repository_folder() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("main-project");
        let metadata = main.join(".git").join("worktrees").join("feature");
        let worktree = temp.path().join("feature-checkout");
        std::fs::create_dir_all(&metadata).unwrap();
        std::fs::create_dir_all(worktree.join("src")).unwrap();
        std::fs::write(metadata.join("commondir"), "../..").unwrap();
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", metadata.display()),
        )
        .unwrap();
        assert_eq!(
            resolve(&worktree.join("src").to_string_lossy(), None),
            "main-project"
        );
    }

    #[test]
    fn claude_worktree_maps_to_main_repository() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("main-project");
        let worktree = main
            .join(".claude")
            .join("worktrees")
            .join("feature")
            .join("src");
        std::fs::create_dir_all(main.join(".git")).unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::create_dir_all(worktree.parent().unwrap().join(".git")).unwrap();
        assert_eq!(resolve(&worktree.to_string_lossy(), None), "main-project");
        assert_eq!(
            resolve(
                "Z:\\missing\\main-project\\.claude\\worktrees\\feature\\src",
                None
            ),
            "main-project"
        );
    }

    #[test]
    fn missing_paths_use_repository_then_lexical_basename() {
        for repository in [
            "https://github.com/team/repo-name.git",
            "git@github.com:team/repo-name.git",
            "ssh://git@github.com/team/repo-name.git",
        ] {
            assert_eq!(
                resolve("Z:\\missing\\checkout", Some(repository)),
                "repo-name"
            );
        }
        assert_eq!(
            resolve("Z:\\missing\\Windows Project\\", None),
            "Windows Project"
        );
        assert_eq!(
            resolve("/missing/posix-project/sub/..", None),
            "posix-project"
        );
        assert_eq!(resolve("", None), "");
        assert_eq!(resolve("C:\\", None), "");
        assert_eq!(resolve("/", None), "");
    }
}
