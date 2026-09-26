# Quota Control

Track your AI coding subscriptions from the Windows and Linux system tray.

Quota Control shows how much of your AI coding plans you have used: session and weekly limits,
credits, and local spend, all in one popup that opens when you click the tray icon.

> **Unofficial port.** Quota Control is an independent Windows and Linux port of
> [OpenUsage](https://github.com/robinebers/openusage) by Robin Ebers, which is a native macOS app.
> It is not the official OpenUsage, and it is not affiliated with or endorsed by its author.
> The source code is reused under the MIT license; the OpenUsage name and logo are not used,
> following the upstream [trademark policy](https://github.com/robinebers/openusage/blob/main/TRADEMARK.md).

## Status

Under active development. The port follows the upstream Swift edition (v0.7.12) feature by feature:

| Stage | Scope |
|---|---|
| 1 | Tray icon, popup anchored to the tray, footer, light and dark themes |
| 2 | Claude and Codex limits, pacing, local spend, Total Spend ring |
| 3 | Cursor, Grok, OpenCode, usage trend, per-model breakdown |
| 4 | Customize and Settings |
| 5 | Copilot, Antigravity, Devin, Ollama, OpenRouter, Z.ai |
| 6 | CLI, local HTTP API, proxy, quota notifications, global shortcut, launch at login |
| 7 | Windows installer, Linux `.deb` and `.AppImage` |

## Stack

- [Tauri 2](https://tauri.app/): a Rust core and the operating system's own web view.
- Rust core: credential readers, provider clients, local log scanners, model pricing, caching.
- React and TypeScript for the popup interface.

## Local backend

The backend runs on the user's computer; no application server is required.
Claude and Codex usage clients feed the Rust refresh engine through Tauri IPC.
Failed requests keep the last successful snapshot alongside an error. Refreshes
have timeouts, backoff, and isolated per-provider state.

Connected accounts are stored independently and remain in the catalog when
offline or signed out. Importing the current CLI login copies its credentials
without changing the CLI's selected account. Imported sessions are read-only;
their refresh tokens are never rotated by this app. Connect through the app's
browser OAuth flow for an independently renewable session. A revoked session
still requires reconnecting; the account card is retained until explicitly removed.
Windows protects credential documents with per-user DPAPI. Linux uses owner-only
directories and files (0700/0600). Credentials are never returned through IPC.

Local token history is shown once per provider family, separately from account
quota cards. It covers this machine's logs, not every device or a particular
account. Input includes cached reads and cache creation; those cache counters
are subsets. Output includes generated reasoning when present in the source's
output count. Missing breakdowns and unknown model prices remain unavailable.
Costs are explicitly estimated API-equivalent usage, not subscription charges.
See [scanner accounting](crates/uc-logscan/README.md) and
[pricing provenance](crates/uc-pricing/README.md).

### Development and validation

```sh
pnpm install
pnpm tauri dev
cargo test --workspace
pnpm build
```

The native host requires the platform prerequisites for Tauri 2 (Windows C++
build tools/WebView2, or Linux GTK/WebKit development libraries). To inspect the
local credential/API/log adapters without opening the UI, run:

```sh
cargo run -p quota-control -- --diagnose
cargo run -p uc-logscan --example diagnostics
```

Diagnostics print sanitized aggregate status, never credential values or prompt
content. The first command reads current CLI credentials without importing them
into the connected-account registry or renewing them. `USAGE_CONTROL_HOME`
redirects app settings/cache/account storage; `CODEX_HOME` and `CLAUDE_CONFIG_DIR`
select source credentials and logs. Browser `pnpm dev` uses fixtures; native
Tauri uses the real backend.

### Frontend contract

`src/lib/backend.ts` is the frontend boundary. Existing provider commands are
`catalog`, `engine_state`, `refresh`, and `set_enabled_providers`. The account
commands are `list_accounts`, `import_current_account`, `begin_account_login`,
`complete_account_login`, `cancel_account_login`, and `remove_account`.
After adding/removing an account the host publishes `catalog-changed` and a new
`engine-state`; no application restart is needed. Subscribe to both events.

`begin_account_login` returns an authorization URL and a ten-minute flow ID.
Open that URL through `open_url`. For Codex (`callbackMode: "loopback"`), call
`complete_account_login` without a callback and await browser completion. For
Claude (`callbackMode: "manual"`), pass the returned code/state or callback URL.
Cancel abandoned flows to release their loopback listener. The account list
contains metadata only; render expired/offline accounts rather than filtering
them out because a snapshot is missing.

Linux tray integrations do not all expose click positions. The tray's **Show
Quota Control** menu remains available when direct left-click events are absent.

### Embedded official chat

The tray menu includes **New Claude sign-in**, **New ChatGPT sign-in**, and
**Saved chat sessions** (Vietnamese by default). Each session opens the official
website in its own native window and persists a separate WebView profile under
the app configuration directory. Closing a window preserves its session; opening
an existing session focuses its window or reuses its saved profile. Session rows
are independent of quota refresh and remain visible while offline.

`list_chat_sessions`, `create_chat_session`, and `open_chat_session` expose the
same workflow to the popup. Creation persists metadata before opening the window;
if window creation fails, the saved session remains available for retry. A
`chat-sessions-changed` event targets the popup after creation.
`TauriBackend.onChatSessionsChanged(listener)` subscribes to that event and
replays the saved session list after the listener is attached. The capability is
optional on `Backend` so browser-only implementations may reload on popup show.

Website authentication is separate from quota OAuth. A profile label does not
verify which website account is signed in, and the app never injects OAuth tokens
or copies browser cookies. The official website controls login and expiry.
Google prohibits OAuth in embedded WebViews, so its sign-in button can be blocked;
other methods depend on the website and account policy. Browser sign-in does not
transfer its cookies into an embedded session. See
[Google's browser policy](https://developers.google.com/identity/protocols/oauth2/policies#use-secure-browsers).

Chat windows and their authentication popups receive no native command access.
The host rejects application invokes from every window except the bundled popup,
retains Tauri's remote-origin checks, and blocks chat navigation to native/local
origins. Authentication popups inherit their parent's WebView environment.

On Windows, `cargo run -p quota-control --example profile_smoke` runs a bounded
native WebView2 check with hidden windows and temporary profiles. It verifies
cookie/localStorage isolation, reopening persistence, and popup inheritance using
controlled local pages. It does not validate login to the real AI websites.

## Command line

```sh
usagectl                 # every enabled provider
usagectl claude          # one family, or an exact card id such as codex@1a2b...
usagectl codex --force   # refresh even when the cached snapshot is still fresh
```

`usagectl` prints the same `openusage.limits.v1` JSON as the local HTTP API and exits.
It shares the app's accounts, settings and snapshot cache, reuses snapshots younger than
five minutes, and works while the app is closed. Exit code 0 means every read succeeded, 2 an
invalid argument or unknown provider, and 4 a failed refresh: the JSON is still printed, with the
failure in its `errors` and a warning on stderr. Output is ASCII-escaped, so Windows PowerShell 5.1
parses it correctly.

**Settings → Command line → Install** puts the command on PATH:

- Windows copies `usagectl.exe` from the installation into `%LOCALAPPDATA%\UsageControl\bin` and
  adds that folder to the user PATH; open a new terminal afterwards. The app keeps the copy current
  after upgrades, and uninstalling the app removes both the copy and the PATH entry.
- The Linux `.deb` installs `/usr/bin/usagectl` itself, so Settings shows it as installed with the package.
- The AppImage writes a small launcher, `~/.local/bin/usagectl`, that runs the AppImage with `--cli`.

## Local HTTP API

While the app runs it serves the same routes as OpenUsage on `http://127.0.0.1:6736`, loopback only:

| Route | Returns |
|---|---|
| `GET /v1/limits` | `openusage.limits.v1` for every enabled provider that reports limits |
| `GET /v1/limits/{provider}` | one family or card id; `404 provider_not_found` for anything else |
| `GET /v1/usage`, `GET /v1/usage/{provider}` | the legacy snapshot shape, including local history cards |

Responses allow any origin (CORS `*`), as upstream does, so any program or web page on this computer
can read your limits while the app runs. More than 16 simultaneous connections receive
`503 server_busy`. If the port is already taken, the app keeps running without the API.

## Global shortcut

**Settings → General → Global shortcut** records a key combination that shows or hides the popup
from any app. It needs at least one of Ctrl, Alt, Shift or the Windows key, unless it is a function
key (F1–F24). A combination another app already holds is refused with a message, and ✕ clears it.
The shortcut is released while it is being recorded and registered again when the app starts.

## Installers

Build on the platform you are packaging for. The installers are not code-signed, so Windows
SmartScreen asks for confirmation the first time.

**Windows** (per-user; no administrator rights):

```sh
pnpm install
pnpm tauri build
```

The result is `target/release/bundle/nsis/Quota Control_0.1.0_x64-setup.exe`. It installs into
`%LOCALAPPDATA%\Quota Control` with `usagectl.exe` next to the app and adds a Start menu entry.
`/S` installs or uninstalls silently. Uninstalling removes the `usagectl` PATH entry and copy,
and the launch-at-login entry when it points at this installation. Accounts and settings stay in
`%APPDATA%\UsageControl` and caches in `%LOCALAPPDATA%\UsageControl`, even when **Delete the
application data** is ticked, because that option only clears the web view data; delete those
folders to remove them.

**Linux** (Ubuntu 24.04 or newer, including WSL):

```sh
sudo apt install build-essential curl wget file libssl-dev libwebkit2gtk-4.1-dev libxdo-dev \
  libayatana-appindicator3-dev librsvg2-dev patchelf rustup
rustup default stable
pnpm install
pnpm tauri build
```

The results are `target/release/bundle/deb/Quota Control_0.1.0_amd64.deb` and
`target/release/bundle/appimage/Quota Control_0.1.0_amd64.AppImage`. The `.deb` depends on
WebKitGTK 4.1, GTK 3 and Ayatana AppIndicator, and installs `/usr/bin/quota-control` and
`/usr/bin/usagectl`. The AppImage carries its own libraries and does not need libfuse2; run
`Quota Control_0.1.0_amd64.AppImage --cli` for the command line. Where FUSE is unavailable,
`APPIMAGE_EXTRACT_AND_RUN=1` runs it without mounting.

## Upgrade compatibility

Quota Control retains the previous application storage directories, credential protection,
WebView profiles and application identifier so existing accounts and settings remain available.
The `usagectl` integration command also remains supported. Legacy names in these stable
identifiers are intentional; the application and build executable are named Quota Control
and `quota-control` respectively.

## License

[MIT](LICENSE). Original work copyright Robin Ebers; port copyright Bui Dang Minh.
