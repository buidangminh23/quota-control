# Quota Control

Track your AI coding subscriptions from the macOS menu bar and the Windows and Linux system tray.

Quota Control shows how much of your AI coding plans you have used: session and weekly limits,
credits, and local spend, all in one popup that opens when you click the tray icon. On macOS the
starred metrics also sit around the notch as a Dynamic Island and on the desktop as a widget.

> **Unofficial port.** Quota Control is an independent port of
> [OpenUsage](https://github.com/robinebers/openusage) by Robin Ebers, which is a native macOS app,
> to Windows and Linux, and back to macOS with a Dynamic Island and a desktop widget.
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
| 8 | Signed in-app updates and the release workflow |
| 9 | macOS: menu bar strip, Dynamic Island, desktop widgets in three styles, DMG, one-line installer and Homebrew cask |

## Install on macOS

Apple Silicon, macOS 14 (Sonoma) or later.

```sh
curl -fsSL https://raw.githubusercontent.com/buidangminh23/quota-control/main/scripts/install-macos.sh | bash
```

The script downloads the latest release, checks it against the release's `SHA256SUMS`, installs
`Quota Control.app` into `/Applications` (or `~/Applications`), registers its widgets and opens it.
Run it again to reinstall; `--uninstall` moves the app to the Trash, removes its login item and
`usagectl` link, and keeps accounts and settings.

With Homebrew, from the [buidangminh23/tap](https://github.com/buidangminh23/homebrew-tap):

```sh
brew tap buidangminh23/tap
brew trust --tap buidangminh23/tap
brew install --cask quota-control
```

Or download `Quota-Control_<version>_aarch64.dmg` from the
[latest release](https://github.com/buidangminh23/quota-control/releases/latest) and drag the app to
Applications. The app is signed ad hoc, not notarized by Apple, so the first launch of a copy
downloaded with a browser is blocked: open **System Settings → Privacy & Security** and choose
**Open Anyway**. The script and the Homebrew cask avoid that step, and in-app updates never hit it.

After that the app updates itself like the Windows and Linux versions (see [Updates](#updates)).

### macOS specifics

- **Menu bar.** The app has no Dock icon. Each account shows its mark with its readings, as upstream
  draws them (marks at 16 points, one value at 12 points or two stacked at 9), or the Bars glyph,
  always as template images that turn white on a dark menu bar and black on a light one.
  **Settings → Menu Bar** picks what it lists (the Limits tab's accounts, the starred metrics, or a
  hand-picked set) and whether each account shows one reading or two stacked.
- **Dynamic Island.** On a MacBook with a notch, a black island hugs the notch and its two wings
  carry a reading each, as a percentage, a ring or a bar. Hovering opens the details (or a click,
  when **Expand on Hover** is off): the accounts the island lists, with their meters, plans, emails
  and reset countdowns, and a line for an account that is signed out. A click on the open island
  opens the popup right below it. It also opens by itself for a few seconds when a limit is about to
  run out or a Codex reset is announced. Screens without a notch get the same island as a pill in
  the middle of the menu bar. **Settings → Dynamic Island** chooses the style, the metric beside
  each side of the notch, what the details list and which parts of an account they show.
- **Desktop widgets.** Right-click the desktop, choose **Edit Widgets** and search for Quota Control.
  There are three styles, each in small, medium, large and extra large: **Details** (meters,
  headlines and live reset countdowns), **Rings** (a percentage ring per metric) and **Compact**
  (one line per metric, the most accounts at once). Each fits as many accounts as its size holds and
  says how many it left out. **Settings → Desktop Widget** picks what they list and which parts of an
  account they show. The app writes the readings to
  `~/Library/Application Support/usage-control/widget/glance.json` and asks WidgetKit to reload
  when a reading changes; the sandboxed widgets may read only that folder, not the accounts beside it. While the app is closed the
  widgets keep their last readings and mark them once they are over 20 minutes old.
- **Claude Code login.** Claude Code on macOS keeps its login in the login keychain, not in
  `~/.claude/.credentials.json`. Quota Control reads the `Claude Code-credentials` item through
  `/usr/bin/security`, the tool Claude Code writes it with, so macOS asks nothing; the file is only a
  fallback. Codex keeps using `~/.codex/auth.json`.
- **Files.** Settings, accounts and the widget file live in `~/Library/Application Support/usage-control`,
  caches in `~/Library/Caches/usage-control`, the log in `~/Library/Logs/usage-control`, and
  `usagectl` is linked into `~/.local/bin`. When no shell profile names that folder, the app adds
  it to `~/.zprofile` (or `~/.bash_profile` for bash) in a block marked "Quota Control: usagectl",
  which uninstalling removes.
## Stack

- [Tauri 2](https://tauri.app/): a Rust core and the operating system's own web view.
- Rust core: credential readers, provider clients, local log scanners, model pricing, caching.
- React and TypeScript for the popup interface.

## Local backend

The backend runs on the user's computer; no application server is required.
Claude and Codex usage clients feed the Rust refresh engine through Tauri IPC.
Failed requests keep the last successful snapshot alongside an error. Refreshes
have timeouts, backoff, and isolated per-provider state.

Claude Code and the Codex CLI signed in on this computer appear as accounts
automatically. Their cards read the CLI's current login at every refresh and are
never copied into the account registry, so signing a CLI in, out, or into another
account updates the cards within 30 seconds and whenever the popup opens. The app
never renews a CLI's tokens; the CLI does that itself when it runs.

**Sign In with Google** on the Accounts screen opens the provider's sign-in page in
Google Chrome (the default browser when Chrome is missing). The page returns to a
loopback listener on this computer, and the app saves the account by itself, with
no code to copy. That session renews independently of the CLI; when it belongs to
the same account as a CLI login, it replaces that CLI's card. Accounts imported
by earlier versions stay read-only. Connected accounts remain in the catalog when
offline or signed out; a revoked session still requires reconnecting, and the
account card is retained until explicitly removed.

Cards the app has never seen start enabled. `knownProviders` in `settings.json`
records the cards seen so far, so a card hidden in Customize stays hidden when its
login returns, and a CLI login that appears while the app is closed still shows up.
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
build tools/WebView2, Linux GTK/WebKit development libraries, or Xcode on macOS, whose Swift
compiler builds the Dynamic Island and the widget). To inspect the
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
commands are `list_accounts`, `begin_account_login`, `reopen_account_login`,
`cancel_account_login`, and `remove_account`.
After adding/removing an account the host publishes `catalog-changed` and a new
`engine-state`; no application restart is needed. Subscribe to both events.

`begin_account_login(provider, language)` opens the sign-in page itself and
returns a ten-minute flow ID plus the browser it used (`chrome` or `default`).
The host waits for the browser in the background, so the popup may hide in the
meantime. It reports the result as `account-login` with status `connected`,
`failed`, `cancelled` or `expired`, and shows the popup again for the first two.
`reopen_account_login` shows the same page again, and `cancel_account_login`
releases the loopback listener. The account list contains metadata only
(`credentialMode` is `cli` for a CLI login); render expired/offline accounts
rather than filtering them out because a snapshot is missing.

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

The app puts the command on PATH by itself every time it starts; there is no setting for it:

- Windows copies `usagectl.exe` from the installation into `%LOCALAPPDATA%\UsageControl\bin` and
  adds that folder to the user PATH; terminals opened afterwards find it. The copy is refreshed
  after upgrades, and uninstalling the app removes both the copy and the PATH entry.
- The Linux `.deb` installs `/usr/bin/usagectl` itself, so the app leaves it alone.
- The AppImage writes a small launcher, `~/.local/bin/usagectl`, that runs the AppImage with `--cli`.

A file Quota Control did not create is never replaced, and development builds leave the
installed command alone.

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

## Updates

Installed copies update themselves from this repository's GitHub releases.

- About 20 seconds after launch, and then every six hours while **Settings → App Updates → Check
  for Updates Automatically** is on, the app reads `latest.json` from the latest release. A newer
  version shows a card at the top of the popup with **Install Update** and **What's New**, and the
  tray menu offers **Install Update X…**. A failed background check stays silent; **Check Now** in
  Settings and **Check for Updates…** in the tray menu always report their result.
- **Install Update** downloads the package that matches how the app was installed, verifies its
  signature, installs it and reopens the app, which confirms the new version with a notification.
  Windows runs the NSIS setup in passive mode: a progress window, no questions, no administrator
  rights. A `.deb` asks for an administrator password (through `pkexec`, or a zenity or kdialog
  prompt). An AppImage replaces its own file. On macOS the `.app` bundle is replaced in place,
  asking for an administrator password only when its folder is not writable.
- Every package must carry a minisign signature from the release key whose public half is built
  into the app (`plugins.updater.pubkey` in `src-tauri/tauri.conf.json`), and the signature must
  name the version being installed, so an older signed package cannot be passed off as an update.
  This is separate from Windows code signing: SmartScreen still warns about a new setup file.
- Downloads use the proxy in `~/.usage-control/config.json` when one is set, like provider
  requests, and the system proxy otherwise.
- A development build (`pnpm tauri dev`) cannot update itself; Settings links to the releases page.

## Installers

Build on the platform you are packaging for. The installers are not code-signed, so Windows
SmartScreen asks for confirmation the first time.

**Windows** (per-user; no administrator rights):

```sh
pnpm install
pnpm tauri build
```

The result is `target/release/bundle/nsis/Quota Control_<version>_x64-setup.exe`. It installs into
`%LOCALAPPDATA%\Quota Control` with `usagectl.exe` next to the app and adds a Start menu entry.
`/S` installs or uninstalls silently. Uninstalling removes the `usagectl` PATH entry and copy,
and the launch-at-login entry when it points at this installation. Accounts and settings stay in
`%APPDATA%\UsageControl` and caches in `%LOCALAPPDATA%\UsageControl`, even when **Delete the
application data** is ticked, because that option only clears the web view data; delete those
folders to remove them.

**macOS** (Apple Silicon, Xcode 15 or newer):

```sh
pnpm install
pnpm tauri build
```

`src-tauri/tauri.macos.conf.json` switches the bundles to `app` and `dmg`. `build.rs` compiles the
Swift in `src-tauri/macos/Shared` and `src-tauri/macos/Host` (Dynamic Island, popup window style,
widget reload) into a static library linked into the app, and the bundle step first runs
`node scripts/macos-widget.mjs build`, which builds the WidgetKit extension from
`src-tauri/macos/Shared` and `src-tauri/macos/Widget` and signs it with its sandbox entitlements.
Tauri copies it into `Contents/PlugIns`. The results are
`target/release/bundle/macos/Quota Control.app` and
`target/release/bundle/dmg/Quota Control_<version>_aarch64.dmg`, signed ad hoc
(`APPLE_SIGNING_IDENTITY` switches both the app and the widget to a real identity).

**Linux** (Ubuntu 24.04 or newer, including WSL):

```sh
sudo apt install build-essential curl wget file libssl-dev libwebkit2gtk-4.1-dev libxdo-dev \
  libayatana-appindicator3-dev librsvg2-dev patchelf rustup
rustup default stable
pnpm install
pnpm tauri build
```

The results are `target/release/bundle/deb/Quota Control_<version>_amd64.deb` and
`target/release/bundle/appimage/Quota Control_<version>_amd64.AppImage`. The `.deb` depends on
WebKitGTK 4.1, GTK 3 and Ayatana AppIndicator, and installs `/usr/bin/quota-control` and
`/usr/bin/usagectl`. The AppImage carries its own libraries and does not need libfuse2; run
`Quota Control_<version>_amd64.AppImage --cli` for the command line. Where FUSE is unavailable,
`APPIMAGE_EXTRACT_AND_RUN=1` runs it without mounting.

## Releases

A release is a GitHub release carrying the Windows setup, the `.deb`, the AppImage and the macOS
app archive with their `.sig` files, the macOS DMG, `latest.json` (the manifest installed apps read)
and `SHA256SUMS`. Installed apps see it only once it is published as the latest release, and
publishing refuses a release that lacks the Windows or Linux packages, or the macOS archive once a
published release has carried one (so the releases before the first macOS one do not wait for it).

1. Set the same version in `package.json`, `src-tauri/tauri.conf.json` and `Cargo.toml`
   (`[workspace.package]`), commit, then push a matching tag: `git tag v0.2.0 && git push origin v0.2.0`.
2. The **Release** workflow checks the tag against the three versions, builds and signs the Windows,
   Linux and macOS packages, and uploads everything to a **draft** release.
3. Publish the draft on GitHub, or run `gh release edit v0.2.0 --draft=false --latest`. From then
   on, installed apps offer the update.

Without GitHub Actions, `node scripts/release.mjs local` releases from Windows: it builds the NSIS
setup here and the `.deb` and AppImage in WSL from the committed tree, then assembles them into
`target/release-assets/v0.2.0`. `--publish` uploads that folder to a draft release, and `--latest`
publishes it, then reads `latest.json` back from GitHub and checks every download link.
Publishing creates the tag, and the Release workflow then finds the release published and builds
nothing. `--skip-linux` builds only the Windows setup; such a partial build cannot be published.

On a Mac, `node scripts/release.mjs mac` builds the app bundle and the DMG (signed with
`~/.tauri/quota-control.key` or `TAURI_SIGNING_PRIVATE_KEY`) into `target/release-assets/v0.2.0-macos`;
`--publish` adds them to the draft the other machine uploaded and `--latest` then publishes it.
Every upload merges with the release: `latest.json` keeps the other platforms and `SHA256SUMS` the
other files, so the packages can come from different machines.

### Signing key

Releases are signed with a minisign key made by `pnpm tauri signer generate`. The private key is
kept outside the repository, in `%USERPROFILE%\.tauri\quota-control.key`, where `release.mjs local`
reads it. The workflow reads it from the `TAURI_SIGNING_PRIVATE_KEY` secret:

```powershell
Get-Content "$env:USERPROFILE\.tauri\quota-control.key" -Raw | gh secret set TAURI_SIGNING_PRIVATE_KEY -R buidangminh23/quota-control
```

Keep a backup of the key. Without it, no further update can be signed: a new key needs a new public
key in `tauri.conf.json`, and installed apps only receive that through a manual reinstall. The key
has no password; if one is added, also set the `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secret.

## Upgrade compatibility

Quota Control retains the previous application storage directories, credential protection,
WebView profiles and application identifier so existing accounts and settings remain available.
The `usagectl` integration command also remains supported. Legacy names in these stable
identifiers are intentional; the application and build executable are named Quota Control
and `quota-control` respectively.

## License

[MIT](LICENSE). Original work copyright Robin Ebers; port copyright Bui Dang Minh.
