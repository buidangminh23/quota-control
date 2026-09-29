//! Signing the Claude Code or Codex CLI in from the Accounts screen. Quota Control runs the CLI's
//! own login command, which opens the browser and saves the login where the CLI keeps it; the
//! CLI's card then appears like any CLI login found on this computer. Quota Control never sees the
//! tokens the command exchanges, only when it ends and the sign-in address it prints.

use std::collections::{HashMap, VecDeque};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Notify;
use uc_providers::ProviderKind;

/// How long a login command may wait for the browser before it is stopped.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const KEPT_LINES: usize = 20;
const MAX_LINE: usize = 2_048;
/// The exit status of a Unix shell that could not find the command.
const NOT_FOUND: i32 = 127;

/// Waits for a login command to end.
pub type Ended = std::pin::Pin<Box<dyn std::future::Future<Output = CliLoginEnd> + Send>>;

/// How a login command ended.
#[derive(Debug)]
pub enum CliLoginEnd {
    Finished,
    Failed(String),
    Cancelled,
    Expired,
}

/// The login commands running now, by flow id.
#[derive(Default)]
pub struct CliLogins {
    flows: parking_lot::Mutex<HashMap<String, Arc<Flow>>>,
}

struct Flow {
    kind: ProviderKind,
    cancel: Notify,
    output: parking_lot::Mutex<Output>,
}

#[derive(Default)]
struct Output {
    url: Option<String>,
    lines: VecDeque<String>,
}

impl CliLogins {
    /// Start the CLI's login command. The returned future waits for it to end; the caller runs it
    /// in the background.
    pub fn start(self: &Arc<Self>, kind: ProviderKind) -> Result<(String, Ended), String> {
        if self.flows.lock().values().any(|flow| flow.kind == kind) {
            return Err(format!(
                "{} is already signing in. Finish or cancel that sign-in first.",
                product(kind)
            ));
        }
        let mut child = login_command(kind)?.spawn().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                not_installed(kind)
            } else {
                format!("{} could not start: {error}", product(kind))
            }
        })?;
        let flow = Arc::new(Flow {
            kind,
            cancel: Notify::new(),
            output: parking_lot::Mutex::new(Output::default()),
        });
        let readers: Vec<_> = [
            child
                .stdout
                .take()
                .map(|pipe| tokio::spawn(collect(pipe, flow.clone()))),
            child
                .stderr
                .take()
                .map(|pipe| tokio::spawn(collect(pipe, flow.clone()))),
        ]
        .into_iter()
        .flatten()
        .collect();
        let flow_id = uuid::Uuid::new_v4().to_string();
        self.flows.lock().insert(flow_id.clone(), flow.clone());
        let logins = self.clone();
        let id = flow_id.clone();
        let ended = Box::pin(async move {
            let end = wait(child, &flow, readers).await;
            logins.flows.lock().remove(&id);
            end
        });
        Ok((flow_id, ended))
    }

    pub fn knows(&self, flow_id: &str) -> bool {
        self.flows.lock().contains_key(flow_id)
    }

    /// Stop a login command; its future then ends with [`CliLoginEnd::Cancelled`].
    pub fn cancel(&self, flow_id: &str) {
        if let Some(flow) = self.flows.lock().get(flow_id) {
            flow.cancel.notify_one();
        }
    }

    /// The sign-in address the command printed, to show the page again.
    pub fn url(&self, flow_id: &str) -> Option<String> {
        self.flows
            .lock()
            .get(flow_id)
            .and_then(|flow| flow.output.lock().url.clone())
    }
}

pub fn product(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Claude => "Claude Code",
        ProviderKind::Codex => "Codex CLI",
    }
}

fn not_installed(kind: ProviderKind) -> String {
    format!("{} is not installed on this computer", product(kind))
}

fn login_args(kind: ProviderKind) -> &'static [&'static str] {
    match kind {
        ProviderKind::Claude => &["auth", "login", "--claudeai"],
        ProviderKind::Codex => &["login"],
    }
}

/// Windows: the CLI found on PATH or where its installers put it, run without a console window.
#[cfg(windows)]
fn login_command(kind: ProviderKind) -> Result<Command, String> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let program = program_path(kind).ok_or_else(|| not_installed(kind))?;
    let mut command = Command::new(program);
    command
        .args(login_args(kind))
        .creation_flags(CREATE_NO_WINDOW);
    Ok(piped(command))
}

/// Unix: the CLI through the user's login shell, which has the PATH the CLI was installed on (an app
/// opened from the desktop starts with a short one). The script is fixed; nothing typed goes in it.
#[cfg(unix)]
fn login_command(kind: ProviderKind) -> Result<Command, String> {
    let name = kind.cli();
    let script = format!(
        "command -v {name} >/dev/null 2>&1 || exit {NOT_FOUND}; exec {name} {}",
        login_args(kind).join(" ")
    );
    let shell = std::env::var_os("SHELL")
        .filter(|shell| std::path::Path::new(shell).is_absolute())
        .unwrap_or_else(default_shell);
    let mut command = Command::new(shell);
    command.arg("-l").arg("-c").arg(script).process_group(0);
    Ok(piped(command))
}

/// The shell an app opened from the desktop, which is given no `SHELL`, signs in with: zsh on
/// macOS (its default), else bash, else sh.
#[cfg(unix)]
fn default_shell() -> std::ffi::OsString {
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &["/bin/zsh", "/bin/bash"]
    } else {
        &["/bin/bash"]
    };
    candidates
        .iter()
        .find(|shell| std::path::Path::new(shell).is_file())
        .copied()
        .unwrap_or("/bin/sh")
        .into()
}

fn piped(mut command: Command) -> Command {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false);
    command
}

#[cfg(windows)]
fn program_path(kind: ProviderKind) -> Option<std::path::PathBuf> {
    let home = uc_core::paths::home_dir();
    let mut dirs: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    if let Some(app_data) = std::env::var_os("APPDATA") {
        dirs.push(std::path::PathBuf::from(app_data).join("npm"));
    }
    dirs.push(home.join(".local").join("bin"));
    if kind == ProviderKind::Codex
        && let Some(local) = std::env::var_os("LOCALAPPDATA")
    {
        dirs.push(
            std::path::PathBuf::from(local)
                .join("Programs")
                .join("OpenAI")
                .join("Codex")
                .join("bin"),
        );
    }
    dirs.iter().find_map(|dir| {
        ["exe", "cmd", "bat"]
            .iter()
            .map(|extension| dir.join(format!("{}.{extension}", kind.cli())))
            .find(|candidate| candidate.is_file())
    })
}

async fn wait(
    mut child: Child,
    flow: &Flow,
    readers: Vec<tokio::task::JoinHandle<()>>,
) -> CliLoginEnd {
    let end = tokio::select! {
        status = child.wait() => {
            let _ = tokio::time::timeout(Duration::from_secs(2), join_readers(readers)).await;
            return match status {
                Ok(status) if status.success() => CliLoginEnd::Finished,
                Ok(status) if status.code() == Some(NOT_FOUND) && cfg!(unix) => {
                    CliLoginEnd::Failed(not_installed(flow.kind))
                }
                Ok(status) => CliLoginEnd::Failed(failure(flow, status.code())),
                Err(error) => CliLoginEnd::Failed(format!("{} stopped: {error}", product(flow.kind))),
            };
        }
        () = flow.cancel.notified() => CliLoginEnd::Cancelled,
        () = tokio::time::sleep(LOGIN_TIMEOUT) => CliLoginEnd::Expired,
    };
    stop(&mut child).await;
    end
}

/// The output readers finishing, so the command's last words are kept.
async fn join_readers(readers: Vec<tokio::task::JoinHandle<()>>) {
    for reader in readers {
        let _ = reader.await;
    }
}

/// Why the command failed, in its own last words when it printed any.
fn failure(flow: &Flow, code: Option<i32>) -> String {
    let output = flow.output.lock();
    let said = output
        .lines
        .iter()
        .rev()
        .find(|line| !line.trim().is_empty() && !line.contains("://"))
        .cloned();
    match (said, code) {
        (Some(line), _) => format!("{}: {line}", product(flow.kind)),
        (None, Some(code)) => format!("{} ended with code {code}", product(flow.kind)),
        (None, None) => format!("{} was stopped", product(flow.kind)),
    }
}

/// Stop the command and whatever it started: the npm launcher runs the CLI as its own child.
async fn stop(child: &mut Child) {
    if let Some(pid) = child.id() {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .creation_flags(0x0800_0000)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        #[cfg(unix)]
        {
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &format!("-{pid}")])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
}

/// Keep the command's last lines and the first sign-in address it prints.
async fn collect(pipe: impl AsyncRead + Unpin, flow: Arc<Flow>) {
    let mut lines = BufReader::new(pipe).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let line = plain(&line);
        let mut output = flow.output.lock();
        if output.url.is_none() {
            output.url = sign_in_url(&line);
        }
        if output.lines.len() == KEPT_LINES {
            output.lines.pop_front();
        }
        output.lines.push_back(line);
    }
}

/// `line` without terminal color codes, cut to a sane length.
fn plain(line: &str) -> String {
    let mut text = String::with_capacity(line.len().min(MAX_LINE));
    let mut chars = line.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if !character.is_control() {
            text.push(character);
        }
        if text.len() >= MAX_LINE {
            break;
        }
    }
    text.trim_end().to_string()
}

/// The first `https://` address in `line` that a browser can open.
fn sign_in_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let url: String = line[start..]
        .chars()
        .take_while(|character| {
            !character.is_whitespace() && !matches!(character, '"' | '\'' | '<' | '>' | ')')
        })
        .collect();
    url::Url::parse(&url).ok().map(|_| url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_codes_and_controls_are_dropped_from_the_output() {
        assert_eq!(plain("\u{1b}[1;32mOpen\u{1b}[0m this\r"), "Open this");
        assert_eq!(plain(&"x".repeat(5_000)).len(), MAX_LINE);
    }

    #[test]
    fn the_sign_in_address_is_found_in_the_line_the_cli_prints() {
        assert_eq!(
            sign_in_url(
                "Browse to: https://auth.openai.com/oauth/authorize?client_id=a&state=b now"
            )
            .as_deref(),
            Some("https://auth.openai.com/oauth/authorize?client_id=a&state=b")
        );
        assert_eq!(
            sign_in_url(
                "If the browser didn't open, visit (https://claude.ai/oauth/authorize?code=true)"
            )
            .as_deref(),
            Some("https://claude.ai/oauth/authorize?code=true")
        );
        assert_eq!(sign_in_url("Listening on http://localhost:1455"), None);
        assert_eq!(sign_in_url("no address here"), None);
    }

    #[test]
    fn each_cli_runs_its_own_login_command() {
        assert_eq!(login_args(ProviderKind::Codex), ["login"]);
        assert_eq!(
            login_args(ProviderKind::Claude),
            ["auth", "login", "--claudeai"]
        );
        assert_eq!(product(ProviderKind::Claude), "Claude Code");
    }
}
