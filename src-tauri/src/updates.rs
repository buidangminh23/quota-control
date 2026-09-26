//! In-app updates (upstream `UpdaterController`, which wraps Sparkle).
//!
//! Every GitHub release carries `latest.json` next to its installers and their minisign
//! signatures. The updater plugin verifies each download against the public key in
//! `tauri.conf.json`, and `requireSignedVersion` rejects a manifest that pairs a new version
//! number with an older signed build. A background task checks shortly after launch and then every
//! six hours while the `automaticUpdateChecks` setting is on; a found update shows as the
//! dashboard banner and in the tray menu, never as a window of its own.
//!
//! Installing hands Windows over to the NSIS installer in passive `/UPDATE` mode, which keeps the
//! shortcuts and launch at login and relaunches the app. On Linux the AppImage is replaced in place
//! or the .deb goes through pkexec, and on macOS the `.app` bundle is replaced in place (asking for
//! an administrator password only when its folder is not writable); then the app restarts itself. A marker written before the
//! handover lets the next launch confirm the new version or report an install that never finished.
//! Builds the updater cannot replace (development runs, other packages) report `supported: false`,
//! and the popup links to the releases page instead.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::utils::config::BundleType;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, Updater, UpdaterExt};

use crate::service::{BackendService, safe_error};

/// The first background check waits for startup to settle.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(20);
/// How often the background task looks at the clock. A check is due by wall-clock time, so a
/// computer that slept through the interval checks soon after it wakes.
const SCHEDULER_TICK: Duration = Duration::from_secs(15 * 60);
/// Minimum time between two background checks.
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// Download progress reaches the popup at most this often.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);
/// An older marker belongs to an abandoned install, not to the launch that just happened.
const MARKER_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
/// The settings document key behind "Check for updates automatically"; missing means on.
const AUTOMATIC_CHECKS_KEY: &str = "automaticUpdateChecks";
const STATUS_EVENT: &str = "update-status";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdatePhase {
    /// Nothing checked since launch.
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    /// Verified and handed to the installer; Windows exits, Linux restarts.
    Installing,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FailureStage {
    Check,
    Download,
    Install,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FailureReason {
    /// The release server could not be reached.
    Network,
    /// The release manifest is missing, malformed or has no build for this package.
    Release,
    /// The download does not match the release signature.
    Signature,
    /// The system refused or the user cancelled the elevation an install needs (pkexec).
    Permission,
    Other,
}

impl FailureReason {
    fn of(error: &tauri_plugin_updater::Error) -> Self {
        use tauri_plugin_updater::Error;
        match error {
            Error::Reqwest(_) | Error::Network(_) => Self::Network,
            Error::ReleaseNotFound
            | Error::TargetNotFound(_)
            | Error::TargetsNotFound(_)
            | Error::Serialization(_)
            | Error::Semver(_)
            | Error::UrlParse(_)
            | Error::InvalidUpdaterFormat => Self::Release,
            Error::Minisign(_)
            | Error::Base64(_)
            | Error::SignatureUtf8(_)
            | Error::SignedVersionMismatch { .. }
            | Error::MissingSignedVersion => Self::Signature,
            Error::AuthenticationFailed => Self::Permission,
            Error::Io(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                Self::Permission
            }
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateFailure {
    pub stage: FailureStage,
    pub reason: FailureReason,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableUpdate {
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
}

impl AvailableUpdate {
    fn of(update: &Update) -> Self {
        Self {
            version: update.version.clone(),
            notes: update.body.clone().filter(|notes| !notes.trim().is_empty()),
            published_at: update
                .raw_json
                .get("pub_date")
                .and_then(|value| value.as_str())
                .map(str::to_owned),
        }
    }
}

/// What the popup shows about updates, pushed as `update-status` on every change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// This installation can replace itself (an installed NSIS, deb, AppImage or .app release build).
    pub supported: bool,
    pub current_version: String,
    pub phase: UpdatePhase,
    /// The user started the current check or install. Background checks leave it false, so the
    /// popup only mentions what they found.
    pub manual: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available: Option<AvailableUpdate>,
    pub downloaded: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// When the last check finished (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<UpdateFailure>,
}

impl UpdateStatus {
    fn idle(supported: bool, current_version: String) -> Self {
        Self {
            supported,
            current_version,
            phase: UpdatePhase::Idle,
            manual: false,
            available: None,
            downloaded: 0,
            total: None,
            checked_at: None,
            failure: None,
        }
    }
}

pub struct Updates {
    status: Mutex<UpdateStatus>,
    /// The update the last check found, kept so installing does not fetch the manifest again.
    pending: Mutex<Option<Update>>,
    /// Checks and installs run one at a time.
    operation: tokio::sync::Mutex<()>,
    last_check: Mutex<Option<SystemTime>>,
}

impl Updates {
    pub fn new(app: &AppHandle) -> Self {
        let supported = installer(app).is_some() && app.updater().is_ok();
        Self {
            status: Mutex::new(UpdateStatus::idle(
                supported,
                app.package_info().version.to_string(),
            )),
            pending: Mutex::new(None),
            operation: tokio::sync::Mutex::new(()),
            last_check: Mutex::new(None),
        }
    }

    pub fn status(&self) -> UpdateStatus {
        self.status.lock().clone()
    }

    /// Report how the previous launch's install went, then start the background checks.
    pub fn start(&self, app: &AppHandle) {
        let current = self.status.lock().current_version.clone();
        match relaunch_outcome(PendingInstall::take(), &current, Utc::now()) {
            Some(Relaunch::Updated { from, to }) => {
                tracing::info!(target: "updates", "updated from {from} to {to}");
                announce_update(app, &to);
            }
            Some(Relaunch::Unfinished { to }) => {
                tracing::warn!(target: "updates", "the install of {to} did not finish; still on {current}");
                self.publish(app, |status| {
                    status.phase = UpdatePhase::Failed;
                    status.manual = true;
                    status.failure = Some(UpdateFailure {
                        stage: FailureStage::Install,
                        reason: FailureReason::Other,
                    });
                    status.available = Some(AvailableUpdate {
                        version: to,
                        notes: None,
                        published_at: None,
                    });
                });
            }
            None => {}
        }
        match installer(app) {
            Some(kind) if self.status.lock().supported => {
                tracing::info!(target: "updates", "self-update enabled ({kind})");
            }
            _ => {
                tracing::info!(target: "updates", "self-update off: not an installed NSIS, deb, AppImage or .app release");
                return;
            }
        }
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(FIRST_CHECK_DELAY).await;
            loop {
                let updates = app.state::<Updates>();
                let last = *updates.last_check.lock();
                if automatic_checks(&app) && check_due(last, SystemTime::now()) {
                    updates.check(&app, false).await;
                }
                tokio::time::sleep(SCHEDULER_TICK).await;
            }
        });
    }

    /// Look for a newer release. A background check (`manual == false`) steps aside while
    /// another operation runs, and its failures only reach the log.
    pub async fn check(&self, app: &AppHandle, manual: bool) -> UpdateStatus {
        if !self.status.lock().supported {
            return self.status();
        }
        let _operation = if manual {
            self.operation.lock().await
        } else {
            match self.operation.try_lock() {
                Ok(operation) => operation,
                Err(_) => return self.status(),
            }
        };
        self.check_locked(app, manual).await;
        self.status()
    }

    async fn check_locked(&self, app: &AppHandle, manual: bool) {
        let previous = self.status();
        self.publish(app, |status| {
            status.phase = UpdatePhase::Checking;
            status.manual = manual;
        });
        let result = match updater(app) {
            Ok(updater) => updater.check().await,
            Err(error) => Err(error),
        };
        *self.last_check.lock() = Some(SystemTime::now());
        let checked_at = Some(Utc::now().to_rfc3339());
        match result {
            Ok(Some(update)) => {
                tracing::info!(target: "updates", "{} is available", update.version);
                let offer = AvailableUpdate::of(&update);
                *self.pending.lock() = Some(update);
                self.publish(app, |status| {
                    status.phase = UpdatePhase::Available;
                    status.available = Some(offer);
                    status.checked_at = checked_at;
                    status.failure = None;
                });
            }
            Ok(None) => {
                *self.pending.lock() = None;
                self.publish(app, |status| {
                    status.phase = UpdatePhase::UpToDate;
                    status.available = None;
                    status.checked_at = checked_at;
                    status.failure = None;
                });
            }
            Err(error) => {
                tracing::warn!(target: "updates", "update check failed: {}", safe_error(&error));
                let failure = UpdateFailure {
                    stage: FailureStage::Check,
                    reason: FailureReason::of(&error),
                };
                self.publish(app, |status| {
                    if manual {
                        status.phase = UpdatePhase::Failed;
                        status.failure = Some(failure);
                    } else {
                        status.phase = previous.phase;
                        status.manual = previous.manual;
                    }
                });
            }
        }
    }

    /// Bring this installation to the newest release: check first when nothing is pending, then
    /// download, verify and hand over to the installer. On success Windows exits here (the
    /// installer relaunches the app) and Linux restarts; `Err` leaves the app running with the
    /// failure in the status.
    pub async fn install(&self, app: &AppHandle) -> Result<(), String> {
        if !self.status.lock().supported {
            return Err("This build of Quota Control cannot update itself".into());
        }
        let _operation = self.operation.lock().await;
        if self.pending.lock().is_none() {
            self.check_locked(app, true).await;
        }
        let Some(update) = self.pending.lock().clone() else {
            return match self.status().phase {
                UpdatePhase::Failed => Err("Checking for updates failed".into()),
                _ => Err("Quota Control is already up to date".into()),
            };
        };
        self.publish(app, |status| {
            status.phase = UpdatePhase::Downloading;
            status.manual = true;
            status.downloaded = 0;
            status.total = None;
            status.failure = None;
        });
        let current = self.status.lock().current_version.clone();
        if let Err(error) = PendingInstall::new(&current, &update.version).save() {
            tracing::warn!(target: "updates", "could not record the pending install: {error}");
        }
        let mut downloaded = 0u64;
        let mut reported: Option<Instant> = None;
        let download = update
            .download(
                |chunk, total| {
                    downloaded += chunk as u64;
                    if reported.is_none_or(|at| at.elapsed() >= PROGRESS_INTERVAL) {
                        reported = Some(Instant::now());
                        self.publish(app, |status| {
                            status.downloaded = downloaded;
                            status.total = total;
                        });
                    }
                },
                || {},
            )
            .await;
        let bytes = match download {
            Ok(bytes) => bytes,
            Err(error) => return Err(self.fail(app, FailureStage::Download, &error)),
        };
        let size = bytes.len() as u64;
        self.publish(app, |status| {
            status.phase = UpdatePhase::Installing;
            status.downloaded = size;
            status.total = Some(size);
        });
        tracing::info!(target: "updates", "installing {} over {current}", update.version);
        if cfg!(windows) {
            crate::flush_log();
        }
        let installer = update.clone();
        let installed = tauri::async_runtime::spawn_blocking(move || installer.install(bytes))
            .await
            .map_err(safe_error)?;
        match installed {
            Ok(()) => {
                tracing::info!(target: "updates", "installed {}; restarting", update.version);
                crate::flush_log();
                app.restart()
            }
            Err(error) => Err(self.fail(app, FailureStage::Install, &error)),
        }
    }

    /// Record a failed download or install and drop the offer, so a retry reads the release again
    /// instead of fetching the same file and signature. A failed Windows handover has already
    /// hidden the popup and removed the tray icon, so the app restarts instead and the kept marker
    /// makes the next launch report the failure.
    fn fail(
        &self,
        app: &AppHandle,
        stage: FailureStage,
        error: &tauri_plugin_updater::Error,
    ) -> String {
        let message = safe_error(error);
        if stage == FailureStage::Install && cfg!(windows) {
            app.restart();
        }
        self.pending.lock().take();
        PendingInstall::remove();
        tracing::warn!(target: "updates", "update {stage:?} failed: {message}");
        self.publish(app, |status| {
            status.phase = UpdatePhase::Failed;
            status.failure = Some(UpdateFailure {
                stage,
                reason: FailureReason::of(error),
            });
        });
        message
    }

    /// Apply `change`, tell the popup, and rebuild the tray menu when the offered version changed.
    fn publish(&self, app: &AppHandle, change: impl FnOnce(&mut UpdateStatus)) {
        let (next, offer_changed) = {
            let mut status = self.status.lock();
            let before = status.available.as_ref().map(|offer| offer.version.clone());
            change(&mut status);
            let after = status.available.as_ref().map(|offer| &offer.version);
            (status.clone(), before.as_ref() != after)
        };
        if app.emit_to("popup", STATUS_EVENT, &next).is_err() {
            tracing::warn!(target: "updates", "could not publish the update status");
        }
        if offer_changed && crate::update_tray_menu(app).is_err() {
            tracing::warn!(target: "updates", "could not update the tray menu");
        }
    }

    /// The tray menu entry: the found update, or a manual check. `None` where updates are off.
    pub fn menu_label(&self, english: bool) -> Option<String> {
        let status = self.status.lock();
        if !status.supported {
            return None;
        }
        Some(match &status.available {
            Some(offer) if english => format!("Install Update {}…", offer.version),
            Some(offer) => format!("Cài bản mới {}…", offer.version),
            None if english => "Check for Updates…".to_owned(),
            None => "Kiểm tra phiên bản mới…".to_owned(),
        })
    }
}

/// The package type the updater can replace, when this is one.
fn installer(app: &AppHandle) -> Option<&'static str> {
    match tauri::utils::platform::bundle_type()? {
        BundleType::Nsis => Some("nsis"),
        BundleType::Deb => Some("deb"),
        BundleType::AppImage if running_appimage(app) => Some("appimage"),
        BundleType::App if running_app_bundle() => Some("app"),
        _ => None,
    }
}

/// macOS reports every build as an `.app`; only a release build running from inside a bundle can
/// replace itself.
fn running_app_bundle() -> bool {
    cfg!(target_os = "macos")
        && !cfg!(debug_assertions)
        && std::env::current_exe()
            .ok()
            .and_then(|path| path.to_str().map(replaceable_bundle))
            .unwrap_or(false)
}

/// Whether the updater can replace the bundle the app runs from: one inside an `.app`, not on the
/// mounted DMG (read-only) or a Gatekeeper translocation (a read-only copy in a random folder).
fn replaceable_bundle(executable: &str) -> bool {
    executable.contains(".app/Contents/MacOS/")
        && !executable.starts_with("/Volumes/")
        && !executable.contains("/AppTranslocation/")
}

#[cfg(target_os = "linux")]
fn running_appimage(app: &AppHandle) -> bool {
    app.env().appimage.is_some()
}

#[cfg(not(target_os = "linux"))]
fn running_appimage(_: &AppHandle) -> bool {
    false
}

/// The configured updater, routed through the user's proxy (`~/.usage-control/config.json`) like
/// every provider request; without one it follows the system proxy.
fn updater(app: &AppHandle) -> tauri_plugin_updater::Result<Updater> {
    let mut builder = app.updater_builder().timeout(CHECK_TIMEOUT);
    if let Some(proxy) = uc_core::http::ProxyConfig::current() {
        match proxy.reqwest_proxy() {
            Ok(rule) => {
                builder = builder.configure_client(move |client| client.proxy(rule.clone()))
            }
            Err(error) => {
                tracing::warn!(target: "updates", "ignoring the configured proxy: {}", safe_error(error));
            }
        }
    }
    builder.build()
}

fn automatic_checks(app: &AppHandle) -> bool {
    app.state::<BackendService>()
        .load("settings")
        .ok()
        .flatten()
        .and_then(|settings| {
            settings
                .get(AUTOMATIC_CHECKS_KEY)
                .and_then(|value| value.as_bool())
        })
        .unwrap_or(true)
}

/// A background check is due when none ran yet, the interval passed, or the clock moved back.
fn check_due(last: Option<SystemTime>, now: SystemTime) -> bool {
    last.is_none_or(|last| {
        now.duration_since(last)
            .map_or(true, |since| since >= CHECK_INTERVAL)
    })
}

fn announce_update(app: &AppHandle, version: &str) {
    use tauri_plugin_notification::NotificationExt;
    let body = if crate::english(app) {
        format!("Quota Control was updated to version {version}.")
    } else {
        format!("Đã cập nhật Quota Control lên phiên bản {version}.")
    };
    if let Err(error) = app
        .notification()
        .builder()
        .title("Quota Control")
        .body(body)
        .show()
    {
        tracing::warn!(target: "updates", "could not announce the update: {}", safe_error(error));
    }
}

/// Written just before the installer takes over and read by the next launch.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingInstall {
    from: String,
    to: String,
    started_at: DateTime<Utc>,
}

impl PendingInstall {
    fn new(from: &str, to: &str) -> Self {
        Self {
            from: from.to_owned(),
            to: to.to_owned(),
            started_at: Utc::now(),
        }
    }

    fn path() -> PathBuf {
        uc_core::paths::cache_dir().join("pending-update.json")
    }

    fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory)?;
        }
        let bytes = serde_json::to_vec(self).map_err(std::io::Error::other)?;
        uc_core::paths::write_atomic(&path, &bytes)
    }

    /// Read and delete the marker.
    fn take() -> Option<Self> {
        let path = Self::path();
        let bytes = std::fs::read(&path).ok()?;
        let _ = std::fs::remove_file(&path);
        serde_json::from_slice(&bytes).ok()
    }

    fn remove() {
        let _ = std::fs::remove_file(Self::path());
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Relaunch {
    Updated { from: String, to: String },
    Unfinished { to: String },
}

fn relaunch_outcome(
    marker: Option<PendingInstall>,
    current: &str,
    now: DateTime<Utc>,
) -> Option<Relaunch> {
    let marker = marker?;
    let age = now.signed_duration_since(marker.started_at).to_std().ok()?;
    if age > MARKER_MAX_AGE {
        return None;
    }
    let same = match (
        semver::Version::parse(marker.to.trim_start_matches('v')),
        semver::Version::parse(current),
    ) {
        (Ok(target), Ok(current)) => target == current,
        _ => marker.to == current,
    };
    Some(if same {
        Relaunch::Updated {
            from: marker.from,
            to: marker.to,
        }
    } else {
        Relaunch::Unfinished { to: marker.to }
    })
}

#[tauri::command]
pub fn update_status(updates: State<'_, Updates>) -> UpdateStatus {
    updates.status()
}

#[tauri::command]
pub async fn check_for_update(
    app: AppHandle,
    updates: State<'_, Updates>,
) -> Result<UpdateStatus, String> {
    Ok(updates.check(&app, true).await)
}

#[tauri::command]
pub async fn install_update(app: AppHandle, updates: State<'_, Updates>) -> Result<(), String> {
    updates.install(&app).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_installed_bundle_can_be_replaced() {
        assert!(replaceable_bundle(
            "/Applications/Quota Control.app/Contents/MacOS/quota-control"
        ));
        assert!(replaceable_bundle(
            "/Users/a/Applications/Quota Control.app/Contents/MacOS/quota-control"
        ));
        assert!(!replaceable_bundle(
            "/Volumes/Quota Control/Quota Control.app/Contents/MacOS/quota-control"
        ));
        assert!(!replaceable_bundle(
            "/private/var/folders/x/T/AppTranslocation/1A2B/d/Quota Control.app/Contents/MacOS/quota-control"
        ));
        assert!(!replaceable_bundle("/usr/local/bin/quota-control"));
    }

    #[test]
    fn a_refused_file_operation_reads_as_missing_permission() {
        let denied = tauri_plugin_updater::Error::Io(std::io::Error::from(
            std::io::ErrorKind::PermissionDenied,
        ));
        assert_eq!(FailureReason::of(&denied), FailureReason::Permission);
        let missing =
            tauri_plugin_updater::Error::Io(std::io::Error::from(std::io::ErrorKind::NotFound));
        assert_eq!(FailureReason::of(&missing), FailureReason::Other);
    }

    fn marker(to: &str, minutes_ago: i64) -> PendingInstall {
        PendingInstall {
            from: "0.1.0".into(),
            to: to.into(),
            started_at: Utc::now() - chrono::Duration::minutes(minutes_ago),
        }
    }

    #[test]
    fn a_relaunch_on_the_target_version_confirms_the_update() {
        assert_eq!(
            relaunch_outcome(Some(marker("0.2.0", 2)), "0.2.0", Utc::now()),
            Some(Relaunch::Updated {
                from: "0.1.0".into(),
                to: "0.2.0".into()
            })
        );
        assert_eq!(
            relaunch_outcome(Some(marker("v0.2.0", 2)), "0.2.0", Utc::now()),
            Some(Relaunch::Updated {
                from: "0.1.0".into(),
                to: "v0.2.0".into()
            })
        );
    }

    #[test]
    fn a_relaunch_on_the_old_version_reports_an_unfinished_install() {
        assert_eq!(
            relaunch_outcome(Some(marker("0.2.0", 2)), "0.1.0", Utc::now()),
            Some(Relaunch::Unfinished { to: "0.2.0".into() })
        );
    }

    #[test]
    fn stale_or_missing_markers_are_ignored() {
        assert_eq!(relaunch_outcome(None, "0.1.0", Utc::now()), None);
        assert_eq!(
            relaunch_outcome(Some(marker("0.2.0", 25 * 60)), "0.1.0", Utc::now()),
            None
        );
        assert_eq!(
            relaunch_outcome(Some(marker("0.2.0", -5)), "0.1.0", Utc::now()),
            None
        );
    }

    #[test]
    fn background_checks_follow_the_wall_clock() {
        let now = SystemTime::now();
        assert!(check_due(None, now));
        assert!(!check_due(Some(now - Duration::from_secs(60)), now));
        assert!(check_due(Some(now - CHECK_INTERVAL), now));
        assert!(check_due(Some(now + Duration::from_secs(60)), now));
    }

    #[test]
    fn failures_are_grouped_for_the_popup() {
        use tauri_plugin_updater::Error;
        assert_eq!(
            FailureReason::of(&Error::Network("503".into())),
            FailureReason::Network
        );
        assert_eq!(
            FailureReason::of(&Error::ReleaseNotFound),
            FailureReason::Release
        );
        assert_eq!(
            FailureReason::of(&Error::TargetsNotFound(vec!["linux-x86_64-deb".into()])),
            FailureReason::Release
        );
        assert_eq!(
            FailureReason::of(&Error::MissingSignedVersion),
            FailureReason::Signature
        );
        assert_eq!(
            FailureReason::of(&Error::SignedVersionMismatch {
                signed: "0.1.0".into(),
                announced: "0.2.0".into()
            }),
            FailureReason::Signature
        );
        assert_eq!(
            FailureReason::of(&Error::AuthenticationFailed),
            FailureReason::Permission
        );
        assert_eq!(
            FailureReason::of(&Error::PackageInstallFailed),
            FailureReason::Other
        );
    }

    #[test]
    fn the_status_serializes_in_the_popup_shape() {
        let mut status = UpdateStatus::idle(true, "0.1.0".into());
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            serde_json::json!({
                "supported": true,
                "currentVersion": "0.1.0",
                "phase": "idle",
                "manual": false,
                "downloaded": 0
            })
        );
        status.phase = UpdatePhase::Failed;
        status.available = Some(AvailableUpdate {
            version: "0.2.0".into(),
            notes: None,
            published_at: Some("2026-09-26T08:00:00Z".into()),
        });
        status.failure = Some(UpdateFailure {
            stage: FailureStage::Download,
            reason: FailureReason::Signature,
        });
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["phase"], "failed");
        assert_eq!(
            value["available"],
            serde_json::json!({"version": "0.2.0", "publishedAt": "2026-09-26T08:00:00Z"})
        );
        assert_eq!(
            value["failure"],
            serde_json::json!({"stage": "download", "reason": "signature"})
        );
    }

    #[test]
    fn the_pending_install_marker_round_trips() {
        let marker = marker("0.2.0", 1);
        let text = serde_json::to_string(&marker).unwrap();
        assert!(text.contains("\"startedAt\""));
        assert_eq!(
            serde_json::from_str::<PendingInstall>(&text).unwrap(),
            marker
        );
    }
}
