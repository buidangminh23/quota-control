//! Opening a sign-in page. Google Chrome is preferred, because that is where the owner's Claude and
//! ChatGPT sessions usually live; without Chrome the system's default browser opens the page.

use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::service::{safe_error, validate_url};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LoginBrowser {
    Chrome,
    Default,
}

pub fn open_login_page<R: tauri::Runtime>(
    app: &AppHandle<R>,
    url: &str,
) -> Result<LoginBrowser, String> {
    validate_url(url)?;
    if let Some(chrome) = chrome_path() {
        match Command::new(&chrome).arg(url).spawn() {
            Ok(child) => {
                reap(child);
                return Ok(LoginBrowser::Chrome);
            }
            Err(error) => {
                tracing::warn!("Google Chrome did not start ({error}); using the default browser")
            }
        }
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(safe_error)?;
    Ok(LoginBrowser::Default)
}

/// Windows keeps no zombie processes; elsewhere the launcher's exit status is collected so it does
/// not linger until the app quits.
fn reap(child: std::process::Child) {
    #[cfg(not(windows))]
    {
        let mut child = child;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
    #[cfg(windows)]
    drop(child);
}

#[cfg(windows)]
fn chrome_path() -> Option<PathBuf> {
    registered_chrome().or_else(|| {
        ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"]
            .into_iter()
            .filter_map(std::env::var_os)
            .map(|root| PathBuf::from(root).join(r"Google\Chrome\Application\chrome.exe"))
            .find(|path| path.is_file())
    })
}

/// Chrome's installer records its executable under `App Paths`, per user or for the machine.
#[cfg(windows)]
fn registered_chrome() -> Option<PathBuf> {
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
    };
    let subkey: Vec<u16> = r"Software\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let mut size: u32 = 0;
        let status = unsafe {
            RegGetValueW(
                root,
                subkey.as_ptr(),
                std::ptr::null(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if status != 0 || size < 4 {
            continue;
        }
        let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
        let status = unsafe {
            RegGetValueW(
                root,
                subkey.as_ptr(),
                std::ptr::null(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status != 0 {
            continue;
        }
        let length = buffer
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(buffer.len());
        let path = PathBuf::from(String::from_utf16_lossy(&buffer[..length]));
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn chrome_path() -> Option<PathBuf> {
    let search = std::env::var_os("PATH")?;
    ["google-chrome-stable", "google-chrome"]
        .into_iter()
        .find_map(|name| {
            std::env::split_paths(&search)
                .map(|directory| directory.join(name))
                .find(|candidate| candidate.is_file())
        })
}

#[cfg(not(any(windows, target_os = "linux")))]
fn chrome_path() -> Option<PathBuf> {
    None
}
