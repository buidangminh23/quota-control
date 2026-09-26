use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

pub(crate) fn resolve(cwd: &str, repository: Option<&str>) -> String {
    if let Some(root) = local_root(cwd) {
        return local_repository(&root.join(".git"), &root);
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

pub(crate) fn repository_alias(cwd: &str) -> Option<(String, String)> {
    let root = local_root(cwd)?;
    let metadata = repository_metadata(&root.join(".git"))?
        .canonicalize()
        .ok()?;
    if metadata.file_name()? != ".git" {
        return None;
    }
    let main = metadata.parent()?;
    let alias = main.file_name()?.to_str().and_then(safe_name)?;
    let canonical = origin_name(&metadata.join("config"), 0)?;
    Some((alias, canonical))
}

pub(crate) fn local_root(cwd: &str) -> Option<PathBuf> {
    if cwd.is_empty() {
        return None;
    }
    let path = Path::new(cwd);
    for ancestor in path.ancestors() {
        if ancestor.file_name().is_some_and(|name| name == "worktrees")
            && let Some(parent) = ancestor.parent()
            && parent.file_name().is_some_and(|name| name == ".claude")
            && let Some(main) = parent.parent()
            && main.join(".git").exists()
        {
            return Some(main.to_path_buf());
        }
    }
    path.ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .map(Path::to_path_buf)
}

fn local_repository(marker: &Path, root: &Path) -> String {
    let metadata = repository_metadata(marker);
    metadata
        .as_deref()
        .and_then(|metadata| origin_name(&metadata.join("config"), 0))
        .unwrap_or_else(|| {
            metadata
                .as_deref()
                .filter(|metadata| metadata.file_name().is_some_and(|name| name == ".git"))
                .and_then(Path::parent)
                .map(folder_name)
                .unwrap_or_else(|| folder_name(root))
        })
}

fn repository_metadata(marker: &Path) -> Option<PathBuf> {
    if marker.is_dir() {
        return Some(marker.to_path_buf());
    }
    let content = small_text(marker)?;
    let gitdir = content.trim().strip_prefix("gitdir:")?.trim();
    let gitdir = absolute_from(marker.parent()?, gitdir);
    match small_text(&gitdir.join("commondir")) {
        Some(common) => absolute_from(&gitdir, common.trim()).canonicalize().ok(),
        None => Some(gitdir),
    }
}

fn origin_name(config: &Path, depth: usize) -> Option<String> {
    if depth >= 8 {
        return None;
    }
    let text = bounded_text(config, 65536)?;
    let mut section = String::new();
    let mut name = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = line
                .strip_prefix('[')
                .and_then(|value| value.split_once(']'))
                .map(|(value, _)| value.trim().to_owned())
                .unwrap_or_default();
            continue;
        }
        let Some((key, raw)) = line.split_once('=') else {
            continue;
        };
        let Some(value) = config_value(raw) else {
            continue;
        };
        if section.eq_ignore_ascii_case("include") && key.trim().eq_ignore_ascii_case("path") {
            if let Some(parent) = config.parent()
                && let Some(included) = origin_name(&absolute_from(parent, &value), depth + 1)
            {
                name = Some(included);
            }
        } else if is_origin_section(&section) && key.trim().eq_ignore_ascii_case("url") {
            name = repository_name(&value);
        }
    }
    name
}

fn is_origin_section(section: &str) -> bool {
    if let Some((kind, subsection)) = section.split_once(char::is_whitespace) {
        kind.eq_ignore_ascii_case("remote") && subsection.trim() == "\"origin\""
    } else {
        section.eq_ignore_ascii_case("remote.origin")
    }
}

fn config_value(raw: &str) -> Option<String> {
    let mut value = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for character in raw.trim().chars() {
        if escaped {
            value.push(match character {
                'n' => '\n',
                't' => '\t',
                'b' => '\u{0008}',
                '\\' | '"' => character,
                _ => return None,
            });
            escaped = false;
        } else {
            match character {
                '\\' => escaped = true,
                '"' => quoted = !quoted,
                '#' | ';' if !quoted => break,
                _ => value.push(character),
            }
        }
    }
    (!quoted && !escaped).then(|| value.trim().to_owned())
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
    bounded_text(path, 4096)
}

fn bounded_text(path: &Path, limit: usize) -> Option<String> {
    let mut text = String::new();
    File::open(path)
        .ok()?
        .take((limit + 1) as u64)
        .read_to_string(&mut text)
        .ok()?;
    (text.len() <= limit).then_some(text)
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
    fn local_and_session_repositories_share_the_origin_name() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("bot tele");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(
            root.join(".git/config"),
            "[remote \"origin\"]\nurl = git@github.com:team/bot-tele.git\n",
        )
        .unwrap();
        assert_eq!(resolve(&root.to_string_lossy(), None), "bot-tele");
        assert_eq!(
            repository_alias(&root.to_string_lossy()),
            Some(("bot tele".to_owned(), "bot-tele".to_owned()))
        );
        assert_eq!(
            resolve(
                &root.to_string_lossy(),
                Some("https://github.com/team/obsolete-name.git")
            ),
            "bot-tele"
        );
        assert_eq!(
            resolve(
                "Z:\\missing\\bot tele",
                Some("https://github.com/team/bot-tele.git")
            ),
            "bot-tele"
        );
    }

    #[test]
    fn relative_worktree_metadata_reads_the_common_origin() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("bot tele");
        let metadata = main.join(".git/worktrees/feature");
        let worktree = temp.path().join("feature-checkout");
        std::fs::create_dir_all(&metadata).unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(metadata.join("commondir"), "../..").unwrap();
        std::fs::write(
            worktree.join(".git"),
            "gitdir: ../bot tele/.git/worktrees/feature\n",
        )
        .unwrap();
        std::fs::write(
            main.join(".git/config"),
            "[remote \"origin\"]\nurl = https://github.com/team/bot-tele.git\n",
        )
        .unwrap();
        assert_eq!(resolve(&worktree.to_string_lossy(), None), "bot-tele");
        assert_eq!(
            repository_alias(&worktree.to_string_lossy()),
            Some(("bot tele".to_owned(), "bot-tele".to_owned()))
        );
        assert_eq!(
            resolve(&worktree.join("deleted-child").to_string_lossy(), None),
            "bot-tele"
        );
    }

    #[test]
    fn removed_claude_worktree_uses_the_main_origin() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("bot tele");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(
            root.join(".git/config"),
            "[remote \"origin\"]\nurl = ssh://git@github.com/team/bot-tele.git\n",
        )
        .unwrap();
        assert_eq!(
            resolve(
                &root
                    .join(".claude/worktrees/removed-feature/src")
                    .to_string_lossy(),
                None
            ),
            "bot-tele"
        );
        assert_eq!(
            repository_alias(
                &root
                    .join(".claude/worktrees/removed-feature/src")
                    .to_string_lossy()
            ),
            Some(("bot tele".to_owned(), "bot-tele".to_owned()))
        );
    }

    #[test]
    fn repository_alias_requires_verified_main_metadata_and_origin() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("renamed checkout");
        let nested = root.join("src/deep");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(repository_alias(&nested.to_string_lossy()), None);
        std::fs::create_dir_all(root.join(".git")).unwrap();
        assert_eq!(repository_alias(&nested.to_string_lossy()), None);
        std::fs::write(
            root.join(".git/config"),
            "[remote \"origin\"]\nurl = https://github.com/team/canonical-name.git\n",
        )
        .unwrap();
        assert_eq!(
            repository_alias(&nested.to_string_lossy()),
            Some(("renamed checkout".to_owned(), "canonical-name".to_owned()))
        );
        assert_eq!(
            repository_alias(&temp.path().join("missing checkout/src").to_string_lossy()),
            None
        );
        assert_eq!(repository_alias(""), None);
    }

    #[test]
    fn relative_config_includes_are_bounded_and_support_quoted_values() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("bot tele");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(
            root.join(".git/config"),
            "[include]\npath = \"../remote config\" # relative include\n",
        )
        .unwrap();
        std::fs::write(
            root.join("remote config"),
            "[include]\npath = .git/config\n[remote \"origin\"]\nURL = \"https://github.com/team/bot-tele.git\" ; origin\n",
        )
        .unwrap();
        assert_eq!(resolve(&root.to_string_lossy(), None), "bot-tele");
    }

    #[test]
    fn repository_root_and_nested_paths_use_root_name() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("sample-project");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let nested = root.join("src").join("deep");
        std::fs::create_dir_all(&nested).unwrap();
        for path in [&root, &nested] {
            assert_eq!(resolve(&path.to_string_lossy(), None), "sample-project");
            assert_eq!(
                resolve(
                    &path.to_string_lossy(),
                    Some("https://github.com/team/stale-session-project.git")
                ),
                "sample-project"
            );
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
