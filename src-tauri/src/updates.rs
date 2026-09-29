//! In-app updates (upstream `UpdaterController`, which wraps Sparkle).
//!
//! Every GitHub release carries `latest.json` next to its installers and their minisign
//! signatures. The updater plugin verifies each download against the public key in
//! `tauri.conf.json`, and `requireSignedVersion` rejects a manifest that pairs a new version
//! number with an older signed build. A background task checks shortly after launch and then every
//! hour while the `automaticUpdateChecks` setting is on. With `automaticUpdateInstalls` on as well,
//! a found update installs on its own once the popup has been closed for ten minutes, where
//! replacing the installation asks the user for nothing. Such an install starts only after its
//! marker and the try are saved, so one that fails is tried again hours later and not at every
//! launch.
//! Otherwise the update shows as a dialog in the
//! popup and in the tray menu; while the popup is closed, a system notification says so once per
//! version. The first launch after an update says which version it replaced the same way.
//!
//! Installing hands Windows over to the NSIS installer in passive `/UPDATE` mode, which keeps the
//! shortcuts and launch at login and relaunches the app. On Linux the AppImage is replaced in place
//! or the .deb goes through pkexec, and on macOS the `.app` bundle is replaced in place (asking for
//! an administrator password only when its folder is not writable); then the app restarts itself. A marker written before the
//! handover lets the next launch confirm the new version or report an install that never finished.
//! A launch while that installer still runs steps aside (`update_installing`): the installer
//! relaunches the new version itself, and a running copy would hold the executable it is writing
//! and read the marker meant for the new version.
//! Builds the updater cannot replace (development runs, other packages) report `supported: false`,
//! and the popup links to the releases page instead.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::utils::config::BundleType;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, Updater, UpdaterExt};

use crate::service::{BackendService, safe_error};

#[path = "update_release.rs"]
mod release;

/// The first background check waits for startup to settle.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(20);
/// How often the background task looks at the clock. A check is due by wall-clock time, so a
/// computer that slept through the interval checks soon after it wakes.
const SCHEDULER_TICK: Duration = Duration::from_secs(5 * 60);
/// Minimum time between two background checks.
const CHECK_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// How long the popup must have been closed before a release installs without being asked, so
/// that a sign-in which went on in the browser is not cut off.
const POPUP_REST: Duration = Duration::from_secs(10 * 60);
/// How long after an install nobody asked for failed the same release is tried again.
const UNASKED_RETRY: Duration = Duration::from_secs(6 * 60 * 60);
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// Download progress reaches the popup at most this often.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);
/// An older marker belongs to an abandoned install, not to the launch that just happened.
const MARKER_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
/// The settings document key behind "Check for updates automatically"; missing means on.
const AUTOMATIC_CHECKS_KEY: &str = "automaticUpdateChecks";
/// The settings document key behind "Install updates automatically"; missing means on.
const AUTOMATIC_INSTALLS_KEY: &str = "automaticUpdateInstalls";
/// How far the clock may be corrected backwards while a recorded try still counts.
const CLOCK_SLACK: Duration = Duration::from_secs(60);
/// The version the previous launch ran, next to the pending-install marker.
const LAST_RUN_FILE: &str = "last-version";
/// `productName` in `tauri.conf.json`, which names the updater's installer file.
const PRODUCT_NAME: &str = "Quota Control";
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
    /// Replacing this installation asks the user for nothing, so a found release can install on
    /// its own. A .deb, or a package in a folder this user cannot write to, asks for an
    /// administrator password.
    pub unattended: bool,
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
    /// The version this launch replaced, until the popup has shown it (`acknowledge_update`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_from: Option<String>,
}

impl UpdateStatus {
    fn idle(supported: bool, unattended: bool, current_version: String) -> Self {
        Self {
            supported,
            unattended,
            current_version,
            phase: UpdatePhase::Idle,
            manual: false,
            available: None,
            downloaded: 0,
            total: None,
            checked_at: None,
            failure: None,
            updated_from: None,
        }
    }
}

pub struct Updates {
    status: Mutex<UpdateStatus>,
    /// The verified offer, refreshed before installation to exclude withdrawn or incomplete releases.
    pending: Mutex<Option<Update>>,
    /// Checks and installs run one at a time.
    operation: tokio::sync::Mutex<()>,
    last_check: Mutex<Option<SystemTime>>,
    /// The release a background check last found, so its system notification shows once.
    announced: Mutex<Option<String>>,
    /// When the popup was last open.
    seen: Mutex<Option<Instant>>,
    /// The install nobody asked for that was last started. It stays, across restarts, while
    /// that release is not installed.
    unasked_try: Mutex<Option<UnaskedTry>>,
    /// When the look before an install nobody asked for last failed.
    unasked_look_failed: Mutex<Option<Instant>>,
}

impl Updates {
    pub fn new(app: &AppHandle) -> Self {
        let kind = installer(app);
        let supported = kind.is_some() && app.updater().is_ok();
        let unattended = supported && kind.is_some_and(replaceable_unasked);
        let current = app.package_info().version.to_string();
        let tried = UnaskedTry::read().filter(|tried| {
            let waits = tried.waits_for(&current);
            if !waits {
                UnaskedTry::remove();
            }
            waits
        });
        Self {
            status: Mutex::new(UpdateStatus::idle(supported, unattended, current)),
            pending: Mutex::new(None),
            operation: tokio::sync::Mutex::new(()),
            last_check: Mutex::new(None),
            announced: Mutex::new(None),
            seen: Mutex::new(None),
            unasked_try: Mutex::new(tried),
            unasked_look_failed: Mutex::new(None),
        }
    }

    /// Remember, across restarts, that an install of `version` nobody asked for starts now.
    /// False when the try could not be saved.
    fn record_unasked_try(&self, version: &str) -> bool {
        let tried = UnaskedTry::now(version);
        let saved = tried.save();
        if let Err(error) = &saved {
            tracing::warn!(target: "updates", "could not record the install nobody asked for: {error}");
        }
        *self.unasked_try.lock() = Some(tried);
        saved.is_ok()
    }

    /// The popup is open, or was closed just now.
    pub fn popup_seen(&self) {
        *self.seen.lock() = Some(Instant::now());
    }

    pub fn status(&self) -> UpdateStatus {
        self.status.lock().clone()
    }

    /// Report how the previous launch's install went, then start the background checks.
    pub fn start(&self, app: &AppHandle) {
        let (current, supported) = {
            let status = self.status.lock();
            (status.current_version.clone(), status.supported)
        };
        let relaunch = relaunch_outcome(PendingInstall::take(), &current, Utc::now());
        let last_run = if supported {
            LastRun::replace(&current)
        } else {
            None
        };
        let replaced = match &relaunch {
            Some(Relaunch::Updated { from, .. }) => Some(from.clone()),
            _ => updated_from(last_run.as_deref(), &current),
        };
        if let Some(from) = replaced {
            tracing::info!(target: "updates", "updated from {from} to {current}");
            notify(
                app,
                format!("Đã cập nhật Quota Control lên phiên bản {current}."),
                format!("Quota Control was updated to version {current}."),
            );
            self.publish(app, |status| status.updated_from = Some(from));
        }
        match relaunch {
            Some(Relaunch::Updated { .. }) | None => {}
            Some(Relaunch::Unfinished { to }) => {
                tracing::warn!(target: "updates", "the install of {to} did not finish; still on {current}");
                let known = self
                    .unasked_try
                    .lock()
                    .as_ref()
                    .is_some_and(|tried| tried.version == to);
                if !known {
                    self.record_unasked_try(&to);
                }
                notify(
                    app,
                    format!("Chưa cài được bản {to}. Mở Quota Control để thử lại."),
                    format!("Version {to} didn't install. Open Quota Control to try again."),
                );
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
                if updates.installs_unasked(&app)
                    && let Err(error) = updates.install_found(&app, false).await
                {
                    tracing::warn!(target: "updates", "the update did not install on its own: {error}");
                }
                tokio::time::sleep(SCHEDULER_TICK).await;
            }
        });
    }

    /// Look for a newer release. A background check (`manual == false`) steps aside while
    /// another operation runs, and its failures only reach the log. While it holds an offer it
    /// keeps showing that offer, and keeps it when the look fails.
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

    async fn check_locked(&self, app: &AppHandle, manual: bool) -> Checked {
        let kept = keeps_offer(manual, self.pending.lock().is_some());
        if !kept {
            *self.pending.lock() = None;
            self.publish(app, |status| {
                status.phase = UpdatePhase::Checking;
                status.available = None;
                status.failure = None;
                status.manual = manual;
            });
        }
        let result = match updater(app) {
            Ok(updater) => updater.check().await,
            Err(error) => Err(error),
        };
        *self.last_check.lock() = Some(SystemTime::now());
        let checked_at = Some(Utc::now().to_rfc3339());
        let result = match result {
            Ok(Some(update)) => {
                let http = uc_core::http::ReqwestHttpClient::shared();
                match tokio::time::timeout(
                    CHECK_TIMEOUT,
                    release::verify(http.as_ref(), &update.raw_json, &update.version),
                )
                .await
                {
                    Ok(Ok(())) => Ok(Some(update)),
                    Ok(Err(release::ReadinessError::Incomplete)) => {
                        tracing::info!(target: "updates", "withholding incomplete release {}", update.version);
                        Ok(None)
                    }
                    _ => Err(tauri_plugin_updater::Error::Network(
                        "Release readiness could not be checked".into(),
                    )),
                }
            }
            other => other,
        };
        match result {
            Ok(Some(update)) => {
                tracing::info!(target: "updates", "{} is available", update.version);
                let offer = AvailableUpdate::of(&update);
                let version = offer.version.clone();
                *self.pending.lock() = Some(update);
                self.publish(app, |status| {
                    status.phase = UpdatePhase::Available;
                    status.available = Some(offer);
                    status.checked_at = checked_at;
                    status.failure = None;
                    status.manual = manual;
                });
                let on_its_own = self.status.lock().unattended
                    && automatic_installs(app)
                    && self.unasked_try.lock().is_none();
                let announce = {
                    let mut announced = self.announced.lock();
                    let announce = !on_its_own
                        && should_announce_offer(
                            manual,
                            crate::popup_visible(app),
                            announced.as_deref(),
                            &version,
                        );
                    if !manual {
                        *announced = Some(version.clone());
                    }
                    announce
                };
                if announce {
                    notify(
                        app,
                        format!("Có bản mới {version}. Mở Quota Control để cài."),
                        format!(
                            "Version {version} is available. Open Quota Control to install it."
                        ),
                    );
                }
                Checked::Found
            }
            Ok(None) => {
                *self.pending.lock() = None;
                self.publish(app, |status| {
                    status.phase = UpdatePhase::UpToDate;
                    status.available = None;
                    status.checked_at = checked_at;
                    status.failure = None;
                    status.manual = manual;
                });
                Checked::UpToDate
            }
            Err(error) => {
                tracing::warn!(target: "updates", "update check failed: {}", safe_error(&error));
                if kept {
                    return Checked::Failed;
                }
                let failure = UpdateFailure {
                    stage: FailureStage::Check,
                    reason: FailureReason::of(&error),
                };
                self.publish(app, |status| {
                    if manual {
                        status.phase = UpdatePhase::Failed;
                        status.failure = Some(failure);
                    } else {
                        status.phase = UpdatePhase::Idle;
                        status.manual = false;
                    }
                });
                Checked::Failed
            }
        }
    }

    /// Bring this installation to the newest release: recheck release readiness, then
    /// download, verify and hand over to the installer. On success Windows exits here (the
    /// installer relaunches the app) and Linux restarts; `Err` leaves the app running with the
    /// failure in the status.
    pub async fn install(&self, app: &AppHandle) -> Result<(), String> {
        self.install_found(app, true).await
    }

    /// Whether the release the last check found installs now without being asked.
    fn installs_unasked(&self, app: &AppHandle) -> bool {
        let popup_open = crate::popup_visible(app);
        if popup_open {
            self.popup_seen();
        }
        let offered = {
            let status = self.status.lock();
            if !status.unattended || status.phase != UpdatePhase::Available {
                return false;
            }
            status.available.as_ref().map(|offer| offer.version.clone())
        };
        let recorded = self.unasked_try.lock().clone();
        let tried = recorded.as_ref().and_then(|tried| {
            tried
                .age(Utc::now())
                .map(|age| (tried.version.as_str(), age))
        });
        let closed_for = self.seen.lock().map(|seen| seen.elapsed());
        let look_failed = self.unasked_look_failed.lock().map(|at| at.elapsed());
        look_may_repeat(look_failed)
            && unasked_install_due(
                automatic_checks(app) && automatic_installs(app),
                offered.as_deref(),
                tried,
                popup_open,
                closed_for,
            )
    }

    /// `manual` tells that the user asked for the install. One nobody asked for is remembered
    /// across restarts, so a release that fails to install is not tried again right away, and it
    /// starts only after its marker and the try are saved. When the look before it fails, the
    /// next one waits for [`CHECK_INTERVAL`].
    async fn install_found(&self, app: &AppHandle, manual: bool) -> Result<(), String> {
        if !self.status.lock().supported {
            return Err("This build of Quota Control cannot update itself".into());
        }
        let _operation = self.operation.lock().await;
        let checked = self.check_locked(app, manual).await;
        if !manual {
            *self.unasked_look_failed.lock() = (checked == Checked::Failed).then(Instant::now);
        }
        match checked {
            Checked::Found => {}
            Checked::UpToDate => return Err("Quota Control is already up to date".into()),
            Checked::Failed => return Err("Checking for updates failed".into()),
        }
        let Some(update) = self.pending.lock().clone() else {
            return Err("Quota Control is already up to date".into());
        };
        let current = self.status.lock().current_version.clone();
        let marked = PendingInstall::new(&current, &update.version).save();
        if let Err(error) = &marked {
            tracing::warn!(target: "updates", "could not record the pending install: {error}");
        }
        let recorded = manual || self.record_unasked_try(&update.version);
        if !install_may_start(manual, marked.is_ok() && recorded) {
            if !crate::popup_visible(app) {
                let version = &update.version;
                notify(
                    app,
                    format!("Có bản mới {version}. Mở Quota Control để cài."),
                    format!("Version {version} is available. Open Quota Control to install it."),
                );
            }
            return Err("The install did not start because it could not be recorded".into());
        }
        self.publish(app, |status| {
            status.phase = UpdatePhase::Downloading;
            status.manual = manual;
            status.downloaded = 0;
            status.total = None;
            status.failure = None;
        });
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
        let version = self.pending.lock().take().map(|update| update.version);
        PendingInstall::remove();
        tracing::warn!(target: "updates", "update {stage:?} failed: {message}");
        if let Some(version) = version.filter(|_| !crate::popup_visible(app)) {
            let (vietnamese, english) = match stage {
                FailureStage::Download => (
                    format!("Không tải được bản {version}. Mở Quota Control để thử lại."),
                    format!(
                        "Couldn't download version {version}. Open Quota Control to try again."
                    ),
                ),
                _ => (
                    format!("Chưa cài được bản {version}. Mở Quota Control để thử lại."),
                    format!("Version {version} didn't install. Open Quota Control to try again."),
                ),
            };
            notify(app, vietnamese, english);
        }
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

    /// The popup has shown which version this launch replaced.
    pub fn acknowledge_update(&self, app: &AppHandle) {
        if self.status.lock().updated_from.is_some() {
            self.publish(app, |status| status.updated_from = None);
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
    switched_on(app, AUTOMATIC_CHECKS_KEY)
}

fn automatic_installs(app: &AppHandle) -> bool {
    switched_on(app, AUTOMATIC_INSTALLS_KEY)
}

/// A switch in the settings document that is on until the user turns it off.
fn switched_on(app: &AppHandle, key: &str) -> bool {
    app.state::<BackendService>()
        .load("settings")
        .ok()
        .flatten()
        .and_then(|settings| settings.get(key).and_then(|value| value.as_bool()))
        .unwrap_or(true)
}

/// Whether the release a check found installs now without being asked: the user left automatic
/// installs on, the popup has been closed for [`POPUP_REST`] or was never open, and no install
/// of this release that nobody asked for was tried within [`UNASKED_RETRY`].
fn unasked_install_due(
    wanted: bool,
    offered: Option<&str>,
    tried: Option<(&str, Duration)>,
    popup_open: bool,
    closed_for: Option<Duration>,
) -> bool {
    let Some(offered) = offered else {
        return false;
    };
    wanted
        && !popup_open
        && closed_for.is_none_or(|closed| closed >= POPUP_REST)
        && tried.is_none_or(|(version, since)| version != offered || since >= UNASKED_RETRY)
}

/// What a look for a newer release came back with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Checked {
    Found,
    UpToDate,
    Failed,
}

/// Whether a check keeps showing the offer it holds while it looks again: a background check
/// does, so that a failed look does not take the offer away until the next one.
fn keeps_offer(manual: bool, offered: bool) -> bool {
    !manual && offered
}

/// Whether an install may start. One nobody asked for needs its marker and its try on disk: a
/// failed install restarts the app on Windows, and without them every launch would try the same
/// release again.
fn install_may_start(manual: bool, recorded: bool) -> bool {
    manual || recorded
}

/// Whether the look before an install nobody asked for may be repeated: not within
/// [`CHECK_INTERVAL`] of one that failed, so a computer without a connection asks once an hour
/// and leaves the updater free for the popup.
fn look_may_repeat(failed: Option<Duration>) -> bool {
    failed.is_none_or(|since| since >= CHECK_INTERVAL)
}

/// Whether the updater can replace this installation without asking for an administrator
/// password: a .deb goes through pkexec, and every other package needs this user to be able to
/// write to what the updater replaces.
fn replaceable_unasked(kind: &str) -> bool {
    kind != "deb"
        && installed_package(kind).is_some_and(|package| {
            replaced_paths(kind, &package)
                .iter()
                .all(|path| writable(path))
        })
}

/// The installed package: the AppImage, the `.app` bundle, or the executable.
fn installed_package(kind: &str) -> Option<PathBuf> {
    match kind {
        "appimage" => Some(PathBuf::from(std::env::var_os("APPIMAGE")?)),
        "app" => bundle_of(&std::env::current_exe().ok()?),
        _ => std::env::current_exe().ok(),
    }
}

/// What this user must be able to write to for `package` to be replaced: the folder it sits in
/// and, for an `.app`, the bundle too, because the updater moves the whole bundle out of that
/// folder first.
fn replaced_paths(kind: &str, package: &Path) -> Vec<PathBuf> {
    let folder = package.parent().map(Path::to_path_buf);
    let bundle = (kind == "app").then(|| package.to_path_buf());
    folder.into_iter().chain(bundle).collect()
}

/// The `.app` bundle that holds `executable`.
fn bundle_of(executable: &Path) -> Option<PathBuf> {
    executable
        .ancestors()
        .find(|folder| {
            folder
                .extension()
                .is_some_and(|extension| extension == "app")
        })
        .map(Path::to_path_buf)
}

/// Whether this user can write to `path`. Nothing is created, so no file is left behind and no
/// folder is opened.
#[cfg(unix)]
fn writable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `path` is a NUL-terminated string that outlives the call, and `access` only reads it.
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}

/// Whether this user can create a file in `folder`. The file keeps one name, so a copy that
/// could not be removed is the one the next check writes and removes.
#[cfg(not(unix))]
fn writable(folder: &Path) -> bool {
    let probe = folder.join(".quota-control-write-test");
    let made = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&probe)
        .is_ok();
    if made {
        let _ = std::fs::remove_file(&probe);
    }
    made
}

/// A background check is due when none ran yet, the interval passed, or the clock moved back.
fn check_due(last: Option<SystemTime>, now: SystemTime) -> bool {
    last.is_none_or(|last| {
        now.duration_since(last)
            .map_or(true, |since| since >= CHECK_INTERVAL)
    })
}

/// A system notification in the app's language, for update news the closed popup cannot show.
fn notify(app: &AppHandle, vietnamese: String, english: String) {
    let body = if crate::english(app) {
        english
    } else {
        vietnamese
    };
    crate::system::announce(app, "Quota Control".into(), body, "update");
}

/// A background check found a release: say so with a system notification only while the popup is
/// closed (the open popup shows its dialog) and once per version.
fn should_announce_offer(
    manual: bool,
    popup_open: bool,
    announced: Option<&str>,
    version: &str,
) -> bool {
    !manual && !popup_open && announced != Some(version)
}

/// `previous` when it is an older release than `current`: an update made outside the app (a
/// downloaded installer, a package manager). A first launch, the same version or a downgrade is not.
fn updated_from(previous: Option<&str>, current: &str) -> Option<String> {
    let previous = previous?.trim();
    let older = semver::Version::parse(previous.trim_start_matches('v')).ok()?;
    let newer = semver::Version::parse(current.trim_start_matches('v')).ok()?;
    (older < newer).then(|| previous.to_owned())
}

/// The version the previous launch of an installed build ran.
struct LastRun;

impl LastRun {
    fn path() -> PathBuf {
        uc_core::paths::cache_dir().join(LAST_RUN_FILE)
    }

    /// Read the recorded version and record `current` in its place.
    fn replace(current: &str) -> Option<String> {
        let path = Self::path();
        let previous = std::fs::read_to_string(&path)
            .ok()
            .map(|text| text.trim().to_owned());
        if previous.as_deref() != Some(current) {
            let saved = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| uc_core::paths::write_atomic(&path, current.as_bytes()));
            if let Err(error) = saved {
                tracing::warn!(target: "updates", "could not record the running version: {error}");
            }
        }
        previous
    }
}

/// The install nobody asked for that was last started, kept next to the pending-install marker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnaskedTry {
    version: String,
    at: DateTime<Utc>,
}

impl UnaskedTry {
    fn now(version: &str) -> Self {
        Self {
            version: version.to_owned(),
            at: Utc::now(),
        }
    }

    fn path() -> PathBuf {
        uc_core::paths::cache_dir().join("unasked-update.json")
    }

    fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory)?;
        }
        let bytes = serde_json::to_vec(self).map_err(std::io::Error::other)?;
        uc_core::paths::write_atomic(&path, &bytes)
    }

    fn read() -> Option<Self> {
        let bytes = std::fs::read(Self::path()).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn remove() {
        let _ = std::fs::remove_file(Self::path());
    }

    /// Whether the release is still to be installed over `current`. A version that cannot be
    /// read waits.
    fn waits_for(&self, current: &str) -> bool {
        match (
            semver::Version::parse(self.version.trim_start_matches('v')),
            semver::Version::parse(current.trim_start_matches('v')),
        ) {
            (Ok(tried), Ok(current)) => tried > current,
            _ => self.version != current,
        }
    }

    /// How long ago the try was, by the clock on the wall. `None` for a try further ahead than
    /// [`UNASKED_RETRY`]: it was recorded before the clock was set back, and no longer counts.
    fn age(&self, now: DateTime<Utc>) -> Option<Duration> {
        match now.signed_duration_since(self.at).to_std() {
            Ok(age) => Some(age),
            Err(_) => self
                .at
                .signed_duration_since(now)
                .to_std()
                .is_ok_and(|ahead| ahead <= UNASKED_RETRY + CLOCK_SLACK)
                .then_some(Duration::ZERO),
        }
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

    /// Read the marker and leave it in place.
    fn read() -> Option<Self> {
        let bytes = std::fs::read(Self::path()).ok()?;
        serde_json::from_slice(&bytes).ok()
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

/// The version an update installer is putting in place right now, when this launch should step
/// aside for it: a marker younger than a day names another version, and that version's installer
/// is running.
fn installing_version(
    marker: Option<PendingInstall>,
    current: &str,
    now: DateTime<Utc>,
    installer_running: impl Fn(&str) -> bool,
) -> Option<String> {
    let marker = marker?;
    let age = now.signed_duration_since(marker.started_at).to_std().ok()?;
    if age > MARKER_MAX_AGE {
        return None;
    }
    let target = marker.to.trim_start_matches('v');
    let same = match (
        semver::Version::parse(target),
        semver::Version::parse(current),
    ) {
        (Ok(target), Ok(current)) => target == current,
        _ => target == current,
    };
    if same {
        return None;
    }
    installer_running(&installer_image(target)).then(|| target.to_owned())
}

/// The file name the updater gives a downloaded Windows installer.
fn installer_image(version: &str) -> String {
    format!("{PRODUCT_NAME}-{version}-installer.exe")
}

/// The version being installed when this launch must step aside for its installer, checked
/// before the app starts anything. Only Windows runs a separate installer.
pub fn update_installing() -> Option<String> {
    installing_version(
        PendingInstall::read(),
        env!("CARGO_PKG_VERSION"),
        Utc::now(),
        process_running,
    )
}

/// Whether a process whose executable is named `image` is running.
#[cfg(windows)]
fn process_running(image: &str) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    // SAFETY: the snapshot handle is checked before use and closed once; the entry is a plain
    // struct the API fills, with `dwSize` set as it requires.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = false;
        let mut more = Process32FirstW(snapshot, &mut entry) != 0;
        while more {
            let length = entry
                .szExeFile
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(entry.szExeFile.len());
            if String::from_utf16_lossy(&entry.szExeFile[..length]).eq_ignore_ascii_case(image) {
                found = true;
                break;
            }
            more = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
        found
    }
}

#[cfg(not(windows))]
fn process_running(_image: &str) -> bool {
    false
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

#[tauri::command]
pub fn acknowledge_update(app: AppHandle, updates: State<'_, Updates>) {
    updates.acknowledge_update(&app);
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
    fn a_release_is_found_within_the_hour_it_comes_out() {
        let now = SystemTime::now();
        assert!(!check_due(Some(now - Duration::from_secs(59 * 60)), now));
        assert!(check_due(Some(now - Duration::from_secs(60 * 60)), now));
        assert!(SCHEDULER_TICK <= Duration::from_secs(5 * 60));
    }

    #[test]
    fn a_found_release_installs_on_its_own_while_the_popup_rests() {
        let rested = Some(POPUP_REST);
        assert!(unasked_install_due(true, Some("0.3.14"), None, false, None));
        assert!(unasked_install_due(
            true,
            Some("0.3.14"),
            None,
            false,
            rested
        ));
        assert!(
            !unasked_install_due(true, None, None, false, rested),
            "nothing was found"
        );
        assert!(
            !unasked_install_due(false, Some("0.3.14"), None, false, rested),
            "the user turned automatic installs off"
        );
        assert!(
            !unasked_install_due(true, Some("0.3.14"), None, true, rested),
            "the popup is open"
        );
        assert!(
            !unasked_install_due(
                true,
                Some("0.3.14"),
                None,
                false,
                Some(POPUP_REST - Duration::from_secs(1))
            ),
            "a sign-in may still go on in the browser"
        );
    }

    #[test]
    fn a_release_that_failed_to_install_is_not_tried_again_right_away() {
        let soon = Some(("0.3.14", Duration::from_secs(5 * 60)));
        let later = Some(("0.3.14", UNASKED_RETRY));
        assert!(!unasked_install_due(
            true,
            Some("0.3.14"),
            soon,
            false,
            None
        ));
        assert!(unasked_install_due(
            true,
            Some("0.3.14"),
            later,
            false,
            None
        ));
        assert!(
            unasked_install_due(true, Some("0.3.15"), soon, false, None),
            "a newer release is another try"
        );
    }

    #[test]
    fn a_package_that_needs_a_password_waits_to_be_asked() {
        assert!(!replaceable_unasked("deb"));
        let folder = tempfile::tempdir().unwrap();
        assert!(writable(folder.path()));
        assert!(
            std::fs::read_dir(folder.path()).unwrap().next().is_none(),
            "the check leaves nothing behind"
        );
        assert!(!writable(&folder.path().join("missing")));
    }

    #[test]
    fn an_app_bundle_must_be_writable_like_the_folder_it_sits_in() {
        let bundle = Path::new("/Applications/Quota Control.app");
        assert_eq!(
            replaced_paths("app", bundle),
            [PathBuf::from("/Applications"), bundle.to_path_buf()]
        );
        let image = Path::new("/home/user/Apps/Quota Control.AppImage");
        assert_eq!(
            replaced_paths("appimage", image),
            [PathBuf::from("/home/user/Apps")]
        );
    }

    #[test]
    fn a_try_nobody_asked_for_is_remembered_by_the_clock_on_the_wall() {
        let at = "2026-09-29T09:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let tried = UnaskedTry {
            version: "0.3.15".into(),
            at,
        };
        let text = serde_json::to_string(&tried).unwrap();
        assert_eq!(serde_json::from_str::<UnaskedTry>(&text).unwrap(), tried);
        let later = at + chrono::Duration::hours(2);
        assert_eq!(tried.age(later), Some(Duration::from_secs(2 * 3600)));
        let due = unasked_install_due(
            true,
            Some("0.3.15"),
            tried.age(later).map(|age| ("0.3.15", age)),
            false,
            None,
        );
        assert!(
            !due,
            "two hours after the try, whatever was restarted since"
        );
        assert_eq!(
            tried.age(at - chrono::Duration::seconds(20)),
            Some(Duration::ZERO),
            "a clock corrected by seconds keeps the try"
        );
        assert_eq!(
            tried.age(at - chrono::Duration::hours(7)),
            None,
            "seven hours ahead was recorded before the clock was set back"
        );
    }

    #[test]
    fn a_recorded_try_is_dropped_once_its_release_runs() {
        let tried = UnaskedTry::now("0.3.15");
        assert!(tried.waits_for("0.3.14"));
        assert!(!tried.waits_for("0.3.15"));
        assert!(!tried.waits_for("0.3.16"));
    }

    #[test]
    fn a_look_that_failed_is_not_repeated_within_the_hour() {
        assert!(look_may_repeat(None));
        assert!(
            !look_may_repeat(Some(Duration::from_secs(5 * 60))),
            "without a connection every five minutes would hold the updater"
        );
        assert!(look_may_repeat(Some(CHECK_INTERVAL)));
    }

    #[test]
    fn an_install_nobody_asked_for_waits_for_its_marker() {
        assert!(install_may_start(false, true));
        assert!(
            !install_may_start(false, false),
            "without the marker a failed install would be tried again at every launch"
        );
        assert!(install_may_start(true, false), "the user asked for it");
        assert!(install_may_start(true, true));
    }

    #[test]
    fn a_background_check_keeps_the_offer_it_holds() {
        assert!(keeps_offer(false, true));
        assert!(!keeps_offer(false, false));
        assert!(
            !keeps_offer(true, true),
            "a check the user asked for shows that it is checking"
        );
    }

    #[test]
    fn the_bundle_is_the_app_folder_around_the_executable() {
        assert_eq!(
            bundle_of(Path::new(
                "/Applications/Quota Control.app/Contents/MacOS/quota-control"
            )),
            Some(PathBuf::from("/Applications/Quota Control.app"))
        );
        assert_eq!(bundle_of(Path::new("/usr/local/bin/quota-control")), None);
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
        let mut status = UpdateStatus::idle(true, true, "0.1.0".into());
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            serde_json::json!({
                "supported": true,
                "unattended": true,
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
    fn a_newer_version_than_the_last_launch_is_an_update() {
        assert_eq!(updated_from(Some("0.3.0"), "0.3.1"), Some("0.3.0".into()));
        assert_eq!(
            updated_from(Some("v0.2.2\n"), "0.3.1"),
            Some("v0.2.2".into())
        );
        assert_eq!(updated_from(None, "0.3.1"), None);
        assert_eq!(updated_from(Some("0.3.1"), "0.3.1"), None);
        assert_eq!(updated_from(Some("0.4.0"), "0.3.1"), None);
        assert_eq!(updated_from(Some("garbage"), "0.3.1"), None);
    }

    #[test]
    fn only_background_offers_found_while_the_popup_is_closed_notify_once() {
        assert!(should_announce_offer(false, false, None, "0.3.1"));
        assert!(should_announce_offer(false, false, Some("0.3.0"), "0.3.1"));
        assert!(!should_announce_offer(false, false, Some("0.3.1"), "0.3.1"));
        assert!(!should_announce_offer(false, true, None, "0.3.1"));
        assert!(!should_announce_offer(true, false, None, "0.3.1"));
    }

    #[test]
    fn the_replaced_version_reaches_the_popup() {
        let mut status = UpdateStatus::idle(true, false, "0.3.1".into());
        status.updated_from = Some("0.3.0".into());
        assert_eq!(
            serde_json::to_value(&status).unwrap()["updatedFrom"],
            "0.3.0"
        );
    }

    #[test]
    fn a_launch_steps_aside_while_the_installer_of_another_version_runs() {
        let now = Utc::now();
        let running = |image: &str| image == "Quota Control-0.3.2-installer.exe";
        assert_eq!(
            installing_version(Some(marker("0.3.2", 1)), "0.3.1", now, running),
            Some("0.3.2".into())
        );
        assert_eq!(
            installing_version(Some(marker("v0.3.2", 1)), "0.3.1", now, running),
            Some("0.3.2".into())
        );
        assert_eq!(
            installing_version(Some(marker("0.3.2", 1)), "0.3.1", now, |_: &str| false),
            None
        );
        assert_eq!(
            installing_version(Some(marker("0.3.2", 1)), "0.3.2", now, running),
            None
        );
        assert_eq!(
            installing_version(Some(marker("0.3.2", 25 * 60)), "0.3.1", now, running),
            None
        );
        assert_eq!(installing_version(None, "0.3.1", now, running), None);
    }

    #[test]
    fn the_installer_is_named_after_the_product() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["productName"], PRODUCT_NAME);
        assert_eq!(
            installer_image("0.3.2"),
            "Quota Control-0.3.2-installer.exe"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_running_process_is_found_by_its_executable_name() {
        let own = std::env::current_exe().unwrap();
        let name = own.file_name().unwrap().to_string_lossy().to_uppercase();
        assert!(process_running(&name));
        assert!(!process_running("Quota Control-0.0.0-installer.exe"));
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
