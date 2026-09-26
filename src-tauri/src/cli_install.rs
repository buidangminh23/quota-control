//! Keep `usagectl` on the user's PATH (upstream `CommandLineToolInstaller`). There is no setting:
//! every launch of a release build installs the command or brings it up to date.
//!
//! - Windows copies the bundled console program into `%LOCALAPPDATA%\UsageControl\bin` and adds that
//!   directory, and nothing else, to the user PATH. The install directory also holds the uninstaller,
//!   which must never become a command. The copy is refreshed at launch after an update.
//! - Linux packages already install `/usr/bin/usagectl`. Other builds link it into `~/.local/bin`;
//!   an AppImage gets a small script that runs the AppImage with `--cli`.

use crate::service::safe_error;

pub const COMMAND: &str = "usagectl";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliState {
    /// This build ships no `usagectl` (a development run or an unsupported platform).
    Unavailable,
    /// A package manager already put it on PATH.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Managed,
    Installed,
    NotInstalled,
    /// Something Quota Control did not create already holds the command's name.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Conflict,
}

/// At launch: install the command, or bring the copy up to date with this version. A development
/// run leaves the installed command alone, and a name something else holds is never taken.
pub fn sync_at_launch() {
    if cfg!(debug_assertions) {
        return;
    }
    tauri::async_runtime::spawn_blocking(|| {
        if platform::status() != CliState::NotInstalled {
            return;
        }
        if let Err(error) = platform::install() {
            tracing::warn!("usagectl could not be installed: {error}");
        }
    });
}

/// For the uninstaller: take the command off PATH. Reinstalling the app puts it back at the next
/// launch.
pub fn unregister() -> Result<(), String> {
    platform::uninstall()
}

/// `path_list` with `directory` appended, or `None` when an entry already names it. A trailing
/// separator stays trailing, so removing the entry again restores the value byte for byte.
#[cfg_attr(not(windows), allow(dead_code))]
fn with_entry(path_list: &str, directory: &str, separator: char) -> Option<String> {
    if path_list
        .split(separator)
        .any(|entry| same_directory(entry, directory))
    {
        return None;
    }
    Some(if path_list.is_empty() {
        directory.to_string()
    } else if path_list.ends_with(separator) {
        format!("{path_list}{directory}{separator}")
    } else {
        format!("{path_list}{separator}{directory}")
    })
}

/// `path_list` without the entries naming `directory`, or `None` when none did.
#[cfg_attr(not(windows), allow(dead_code))]
fn without_entry(path_list: &str, directory: &str, separator: char) -> Option<String> {
    let entries: Vec<&str> = path_list.split(separator).collect();
    let kept: Vec<&str> = entries
        .iter()
        .copied()
        .filter(|entry| !same_directory(entry, directory))
        .collect();
    (kept.len() != entries.len()).then(|| kept.join(&separator.to_string()))
}

/// Whether a PATH entry names `directory`: the same text once quotes, trailing slashes, variables
/// and (on Windows) case are set aside, or two spellings of one existing folder, such as a
/// `MINHSP~1` short name or a symlink.
fn same_directory(entry: &str, directory: &str) -> bool {
    let normalize = |value: &str| {
        let value = value.trim().trim_matches('"');
        let value = value.trim_end_matches(['\\', '/']);
        if cfg!(windows) {
            expand_variables(value)
        } else {
            value.to_string()
        }
    };
    if entry.trim().is_empty() {
        return false;
    }
    let (entry, directory) = (normalize(entry), normalize(directory));
    if entry == directory || (cfg!(windows) && entry.to_lowercase() == directory.to_lowercase()) {
        return true;
    }
    matches!(
        (std::fs::canonicalize(&entry), std::fs::canonicalize(&directory)),
        (Ok(one), Ok(two)) if one == two
    )
}

/// Expand `%NAME%` references the way the registry's REG_EXPAND_SZ PATH is read.
fn expand_variables(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(expanded) => out.push_str(&expanded),
                    Err(_) => out.push_str(&rest[start..start + end + 2]),
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The `usagectl` shipped beside the running app, when this build has one.
fn bundled() -> Option<std::path::PathBuf> {
    let name = if cfg!(windows) {
        "usagectl.exe"
    } else {
        COMMAND
    };
    let path = std::env::current_exe().ok()?.parent()?.join(name);
    path.is_file().then_some(path)
}

#[cfg(windows)]
mod platform {
    use std::path::{Path, PathBuf};

    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ,
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    };

    use super::{CliState, bundled, same_directory, with_entry, without_entry};

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn copy_path() -> PathBuf {
        uc_core::paths::bin_dir().join("usagectl.exe")
    }

    fn directory_text() -> String {
        uc_core::paths::bin_dir().to_string_lossy().into_owned()
    }

    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            unsafe { RegCloseKey(self.0) };
        }
    }

    fn environment_key() -> Result<Key, String> {
        let mut key: HKEY = std::ptr::null_mut();
        let name = wide("Environment");
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                name.as_ptr(),
                0,
                KEY_QUERY_VALUE | KEY_SET_VALUE,
                &mut key,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(format!(
                "The user environment could not be opened ({status})."
            ));
        }
        Ok(Key(key))
    }

    /// The user PATH as stored (variables unexpanded) and its registry type.
    fn read_user_path(key: &Key) -> Result<Option<(String, u32)>, String> {
        let name = wide("Path");
        let mut kind = 0_u32;
        let mut size = 0_u32;
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != ERROR_SUCCESS {
            return Err(format!("The user PATH could not be read ({status})."));
        }
        let mut buffer = vec![0_u16; (size as usize).div_ceil(2) + 1];
        let mut size = (buffer.len() * 2) as u32;
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(format!("The user PATH could not be read ({status})."));
        }
        if kind != REG_SZ && kind != REG_EXPAND_SZ {
            return Err("The user PATH has an unexpected type; it was left unchanged.".into());
        }
        let length = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        Ok(Some((String::from_utf16_lossy(&buffer[..length]), kind)))
    }

    fn write_user_path(key: &Key, value: &str, kind: u32) -> Result<(), String> {
        let name = wide("Path");
        let data = wide(value);
        let status = unsafe {
            RegSetValueExW(
                key.0,
                name.as_ptr(),
                0,
                kind,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(format!("The user PATH could not be saved ({status})."));
        }
        let area = wide("Environment");
        let mut result = 0_usize;
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                area.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                5000,
                &mut result,
            )
        };
        Ok(())
    }

    fn on_user_path() -> bool {
        environment_key()
            .and_then(|key| read_user_path(&key))
            .ok()
            .flatten()
            .is_some_and(|(value, _)| {
                value
                    .split(';')
                    .any(|entry| same_directory(entry, &directory_text()))
            })
    }

    /// Same length and write time: `std::fs::copy` keeps the source's write time on Windows, so an
    /// update of the app shows up as a difference.
    fn same_file(one: &Path, two: &Path) -> bool {
        match (std::fs::metadata(one), std::fs::metadata(two)) {
            (Ok(one), Ok(two)) => {
                one.len() == two.len() && one.modified().ok() == two.modified().ok()
            }
            _ => false,
        }
    }

    /// Copy `source` over `target`, even while an old `usagectl` is still running from it: Windows
    /// cannot overwrite a running program, but it can rename it out of the way.
    fn replace(source: &Path, target: &Path) -> Result<(), String> {
        let directory = target.parent().ok_or("Invalid command location.")?;
        std::fs::create_dir_all(directory).map_err(super::safe_error)?;
        if let Ok(entries) = std::fs::read_dir(directory) {
            for entry in entries.flatten() {
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("usagectl.exe.old")
                {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        let staged = directory.join(format!("usagectl.exe.new-{}", std::process::id()));
        std::fs::copy(source, &staged).map_err(super::safe_error)?;
        if std::fs::rename(&staged, target).is_ok() {
            return Ok(());
        }
        let retired = directory.join(format!("usagectl.exe.old-{}", std::process::id()));
        let moved =
            std::fs::rename(target, &retired).and_then(|()| std::fs::rename(&staged, target));
        if let Err(error) = moved {
            let _ = std::fs::remove_file(&staged);
            return Err(super::safe_error(error));
        }
        Ok(())
    }

    pub fn status() -> CliState {
        let Some(source) = bundled() else {
            return CliState::Unavailable;
        };
        let copy = copy_path();
        if copy.is_file() && on_user_path() && same_file(&source, &copy) {
            CliState::Installed
        } else {
            CliState::NotInstalled
        }
    }

    pub fn install() -> Result<(), String> {
        let source = bundled().ok_or("This build of Quota Control does not include usagectl.")?;
        let copy = copy_path();
        if !same_file(&source, &copy) {
            replace(&source, &copy)?;
        }
        let key = environment_key()?;
        match read_user_path(&key)? {
            Some((value, kind)) => {
                if let Some(next) = with_entry(&value, &directory_text(), ';') {
                    write_user_path(&key, &next, kind)?;
                }
            }
            None => write_user_path(&key, &directory_text(), REG_EXPAND_SZ)?,
        }
        Ok(())
    }

    pub fn uninstall() -> Result<(), String> {
        let key = environment_key()?;
        if let Some((value, kind)) = read_user_path(&key)?
            && let Some(next) = without_entry(&value, &directory_text(), ';')
        {
            write_user_path(&key, &next, kind)?;
        }
        let copy = copy_path();
        if copy.exists() {
            let retired = copy.with_extension(format!("exe.old-{}", std::process::id()));
            if std::fs::remove_file(&copy).is_err() {
                std::fs::rename(&copy, &retired).map_err(super::safe_error)?;
            }
        }
        if let Some(directory) = copy.parent() {
            let _ = std::fs::remove_dir(directory);
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use super::{COMMAND, CliState, bundled, same_directory};

    /// First line after the shebang of the AppImage wrapper, so uninstall only removes its own file.
    const MARKER: &str = "# Installed by Usage Control";

    fn link_path() -> PathBuf {
        uc_core::paths::bin_dir().join(COMMAND)
    }

    fn appimage() -> Option<PathBuf> {
        std::env::var_os("APPIMAGE")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_file())
    }

    fn quoted(path: &Path) -> String {
        format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
    }

    fn wrapper(appimage: &Path) -> String {
        format!(
            "#!/bin/sh\n{MARKER}\nexec {} --cli \"$@\"\n",
            quoted(appimage)
        )
    }

    fn on_path(directory: &Path) -> bool {
        let directory = directory.to_string_lossy();
        std::env::var("PATH")
            .unwrap_or_default()
            .split(':')
            .any(|entry| same_directory(entry, &directory))
    }

    enum Link {
        Missing,
        Ours,
        Foreign,
    }

    fn existing(link: &Path) -> Link {
        let Ok(metadata) = std::fs::symlink_metadata(link) else {
            return Link::Missing;
        };
        if metadata.file_type().is_symlink() {
            let target = std::fs::read_link(link).ok();
            return match (target, bundled()) {
                (Some(target), Some(source)) if target == source => Link::Ours,
                (Some(target), _) if target.file_name().is_some_and(|name| name == COMMAND) => {
                    if target.exists() {
                        Link::Foreign
                    } else {
                        Link::Ours
                    }
                }
                _ => Link::Foreign,
            };
        }
        match std::fs::read_to_string(link) {
            Ok(text) if text.lines().nth(1) == Some(MARKER) => Link::Ours,
            _ => Link::Foreign,
        }
    }

    fn current(link: &Path) -> bool {
        match appimage() {
            Some(appimage) => {
                std::fs::read_to_string(link).is_ok_and(|text| text == wrapper(&appimage))
            }
            None => bundled().is_some_and(|source| {
                std::fs::read_link(link).is_ok_and(|target| target == source)
            }),
        }
    }

    pub fn status() -> CliState {
        let link = link_path();
        if appimage().is_none() {
            let Some(source) = bundled() else {
                return CliState::Unavailable;
            };
            if source.parent().is_some_and(on_path) {
                return CliState::Managed;
            }
        }
        match existing(&link) {
            Link::Missing => CliState::NotInstalled,
            Link::Foreign => CliState::Conflict,
            Link::Ours if current(&link) => CliState::Installed,
            Link::Ours => CliState::NotInstalled,
        }
    }

    pub fn install() -> Result<(), String> {
        let link = link_path();
        match status() {
            CliState::Managed | CliState::Installed => return Ok(()),
            CliState::Unavailable => {
                return Err("This build of Quota Control does not include usagectl.".into());
            }
            CliState::Conflict => {
                return Err(format!(
                    "{} already exists and was not installed by Quota Control.",
                    link.display()
                ));
            }
            CliState::NotInstalled => {}
        }
        let directory = link.parent().ok_or("Invalid command location.")?;
        std::fs::create_dir_all(directory).map_err(super::safe_error)?;
        if std::fs::symlink_metadata(&link).is_ok() {
            std::fs::remove_file(&link).map_err(super::safe_error)?;
        }
        match appimage() {
            Some(appimage) => {
                std::fs::write(&link, wrapper(&appimage)).map_err(super::safe_error)?;
                std::fs::set_permissions(&link, std::fs::Permissions::from_mode(0o755))
                    .map_err(super::safe_error)?;
            }
            None => {
                let source =
                    bundled().ok_or("This build of Quota Control does not include usagectl.")?;
                std::os::unix::fs::symlink(source, &link).map_err(super::safe_error)?;
            }
        }
        Ok(())
    }

    pub fn uninstall() -> Result<(), String> {
        let link = link_path();
        if matches!(existing(&link), Link::Ours) {
            std::fs::remove_file(&link).map_err(super::safe_error)?;
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use std::os::unix::fs::PermissionsExt;

        use super::{Link, existing, wrapper};

        fn executable(path: &std::path::Path, text: &str) {
            std::fs::write(path, text).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        #[test]
        fn appimage_wrapper_forwards_arguments_and_only_claims_its_own_file() {
            let root = tempfile::tempdir().unwrap();
            let folder = root.path().join("it's a folder");
            std::fs::create_dir(&folder).unwrap();
            let appimage = folder.join("Quota Control.AppImage");
            executable(&appimage, "#!/bin/sh\nprintf '%s|' \"$@\"\n");
            let link = root.path().join("usagectl");
            executable(&link, &wrapper(&appimage));

            let output = std::process::Command::new("sh")
                .arg(&link)
                .args(["codex", "--force", "two words"])
                .output()
                .unwrap();
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                "--cli|codex|--force|two words|"
            );
            assert!(matches!(existing(&link), Link::Ours));

            executable(&link, "#!/bin/sh\nexec /opt/other/usagectl \"$@\"\n");
            assert!(matches!(existing(&link), Link::Foreign));
            std::fs::remove_file(&link).unwrap();
            assert!(matches!(existing(&link), Link::Missing));
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use super::CliState;

    pub fn status() -> CliState {
        CliState::Unavailable
    }

    pub fn install() -> Result<(), String> {
        Err("usagectl is not available on this platform.".into())
    }

    pub fn uninstall() -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_entries_are_added_once_and_removed_everywhere() {
        let directory = if cfg!(windows) {
            r"C:\Users\A\AppData\Local\UsageControl\bin"
        } else {
            "/home/a/.local/bin"
        };
        let separator = if cfg!(windows) { ';' } else { ':' };
        let join = |parts: &[&str]| parts.join(&separator.to_string());
        let other = if cfg!(windows) {
            r"C:\Tools"
        } else {
            "/usr/bin"
        };
        assert_eq!(
            with_entry("", directory, separator).as_deref(),
            Some(directory)
        );
        for original in [
            other.to_string(),
            format!("{other}{separator}"),
            String::new(),
        ] {
            let added = with_entry(&original, directory, separator).unwrap();
            assert_eq!(
                without_entry(&added, directory, separator).as_deref(),
                Some(original.as_str())
            );
        }
        assert_eq!(
            with_entry(&format!("{other}{separator}"), directory, separator),
            Some(format!("{other}{separator}{directory}{separator}"))
        );
        let both = join(&[other, directory]);
        assert_eq!(with_entry(&both, directory, separator), None);
        let trailing = join(&[other, &format!("{directory}/")]);
        assert_eq!(with_entry(&trailing, directory, separator), None);
        assert_eq!(
            without_entry(&both, directory, separator).as_deref(),
            Some(other)
        );
        assert_eq!(
            without_entry(&join(&[directory, other, directory]), directory, separator).as_deref(),
            Some(other)
        );
        assert_eq!(without_entry(other, directory, separator), None);
        assert!(!same_directory("", directory));
    }

    #[test]
    fn different_spellings_of_one_existing_folder_match() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("sub")).unwrap();
        let directory = root.path().to_string_lossy().into_owned();
        let detour = root.path().join("sub").join("..");
        assert!(same_directory(&detour.to_string_lossy(), &directory));
        let missing = root.path().join("missing").join("..").join("elsewhere");
        assert!(!same_directory(&missing.to_string_lossy(), &directory));
    }

    #[cfg(windows)]
    #[test]
    fn windows_entries_match_case_quotes_and_variables() {
        let home = std::env::var("LOCALAPPDATA").unwrap();
        let directory = format!(r"{home}\UsageControl\bin");
        assert!(same_directory(
            r"%LOCALAPPDATA%\UsageControl\bin",
            &directory
        ));
        assert!(same_directory(
            &format!("\"{}\\\"", directory.to_uppercase()),
            &directory
        ));
        assert!(!same_directory(r"%LOCALAPPDATA%\UsageControl", &directory));
        assert_eq!(
            expand_variables("%NO_SUCH_VARIABLE_UC%\\x"),
            "%NO_SUCH_VARIABLE_UC%\\x"
        );
        assert_eq!(expand_variables("50%"), "50%");
    }
}
