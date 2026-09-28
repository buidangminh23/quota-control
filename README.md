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

- **Menu bar.** The app has no Dock icon. Each account shows its colored mark with its readings
  (marks at 16 points, one value at 12 points or two stacked, each with its window name such as
  `5h` or `week`), drawn in white on a dark menu bar and in black on a light one, or the Bars glyph
  as a template image.
  **Settings → Menu Bar** picks what it lists (quick picks follow the Limits tab or the starred
  metrics; any metric picked by hand makes a custom list) and whether each account shows one reading
  or two stacked.
- **Dynamic Island.** On a MacBook with a notch, a black island hugs the notch and its two wings
  carry a reading each, as a percentage, a ring or a bar. Hovering opens the details (or a click,
  when **Expand on Hover** is off): the accounts the island lists, with their meters, plans, emails
  and reset countdowns, and a line for an account that is signed out. A click on the island's footer
  opens the popup right below it. It also opens by itself for a few seconds when a limit is about to
  run out or a Codex reset is announced. Screens without a notch get the same island as a pill in
  the middle of the menu bar. **Settings → Dynamic Island** chooses the style, what sits beside
  each side of the notch (a metric, the soonest limit to come back, or the Codex reset tracker: the
  announced reset's countdown, the chance of a reset within 24 hours, 3 or 7 days, or the time since
  the last one) and the tabs of the open island: **Limits**, **Codex Resets** and **Coming Back**,
  any of them. With several tabs the open island has a tab bar, and a click on a tab shows that tab
  in full (every account and reading, or the whole reset tracker with its calendar and rhythm),
  using the same vertical reset cards as the app and scrolling when the screen is too short.
  Tabs and the footer remain visible while scrolling; **Stacked** shows the tabs together instead.
  Each tab has its own options: the accounts and metrics of Limits (quick picks follow the Limits
  tab or the stars, any metric can be switched on or off by hand) and what each account shows, the
  parts of the reset tracker, and how many limits coming back are listed.
- **Desktop widgets.** Right-click the desktop, choose **Edit Widgets** and search for Quota Control.
  Three quota styles come in small, medium, large and extra large: **Details** (meters, headlines
  and live reset countdowns), **Rings** (a percentage ring per metric) and **Compact** (one line per
  metric, the most accounts at once); each fits as many accounts as its size holds and says how many
  it left out. **Overview** combines the limits with the Codex reset tracker, while the separate
  widgets keep them apart: **Coming Back** lists the next limits to reset with live countdowns,
  **Codex Resets** starts with the latest reset and follows the app's cards for announcements,
  forecast, calendar, rhythm, statistics and history. Page and section controls keep the full
  tracker accessible in each widget size. **Codex Reset Calendar** starts at the 20-week calendar
  and keeps the other tracker sections available too (from
  [codex-resets.com](https://codex-resets.com), shown while the Reset tab or reset notifications are
  on). **Settings → Desktop Widget** picks the parts the Overview combines and, like the island, has
  options per part: the accounts and metrics of the quota widgets, the parts of the reset tracker
  the reset widgets show, and how many limits Coming Back lists. At every launch the app also stops a widget process left over from an earlier version,
  so the widgets always run the installed version's code. The app writes the readings to
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

Other AI services sign in the same way when their own apps have a browser sign-in:
**Sign In with Google** or **Sign In with GitHub** opens the service's page, and the
app saves the account encrypted beside API keys once the page is done. Google
services (Antigravity) come back to a loopback listener; Kiro, Kilo, Cline
and Copilot use a device sign-in, and Copilot's one-time code is copied to the
clipboard to paste on GitHub's page; Cursor, Codebuff and Ollama finish on their own
site while the app waits. Each row of the Add Account list has a plus button that
opens the first sign-in at once; picking the row shows every way to connect,
including an API key or cookie. These sign-ins are the card's own, so the app renews
them when they rotate (Kiro, Cursor, Cline) and never touches the service's app login.

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

### Other AI services

<!-- services:start -->

Besides Claude and Codex, the `uc-services` crate reads these services. A login is read
from the files the service's own CLI, IDE or desktop app saved, never written and never
renewed when its refresh token rotates; keys and pasted session cookies are saved
encrypted for the current user (DPAPI on Windows) and never leave the computer except to
the service itself. A browser sign-in made on the Accounts screen is saved the same way;
its tokens are the card's own, so the card renews them. Every service is its own cargo
feature; `all` is the default.

| Service | Connects with |
| --- | --- |
| Antigravity | the Antigravity login on this computer; or a Google sign-in |
| Copilot | the GitHub Copilot login on this computer; or a GitHub sign-in; or a classic personal access token (the quota, else this month's premium requests from GitHub billing) |
| Cursor | the Cursor login on this computer; or a sign-in on cursor.com (Google, GitHub or email); or a pasted session cookie (WorkosCursorSessionToken); or an Enterprise team's Admin API key (team spend) |
| Kiro | the Kiro login on this computer; or a Google or GitHub sign-in; or an API key (or `KIRO_API_KEY`) |
| Grok | the Grok CLI login on this computer; or its pasted access token (lasts about a week) |
| OpenCode | the OpenCode login on this computer; or an API key (or `OPENCODE_API_KEY`) |
| Ollama | the Ollama login on this computer; or a sign-in on ollama.com (Google, GitHub or email) that links a key of its own; or an API key (or `OLLAMA_API_KEY`) (card starts hidden) |
| Devin | the Devin login on this computer; or the CLI's API key, or an Enterprise admin's personal key for the organization's ACUs (or `DEVIN_API_KEY`) |
| Zed | the Zed login on this computer |
| Qoder | the Qoder login on this computer; or an API key (or `QODER_PERSONAL_ACCESS_TOKEN`) |
| CodeBuddy | an API key (or `CODEBUDDY_API_KEY`, `CODEBUDDY_AUTH_TOKEN`) |
| Z.ai | an API key (or `ZAI_API_KEY`, `Z_AI_API_KEY`, `GLM_API_KEY`, `ZHIPUAI_API_KEY`) |
| MiniMax | an API key (or `MINIMAX_API_KEY`) |
| Kimi | the Kimi Code login on this computer; or an API key (or `KIMI_API_KEY`, `KIMI_CODE_API_KEY`) |
| DeepSeek | an API key (or `DEEPSEEK_API_KEY`) |
| OpenRouter | an API key (or `OPENROUTER_API_KEY`, `OPENROUTER_KEY`) |
| Groq | an API key (or `GROQ_API_KEY`) |
| Vercel AI Gateway | an API key (or `AI_GATEWAY_API_KEY`) |
| SiliconFlow | an API key (or `SILICONFLOW_API_KEY`) |
| Chutes | an API key (or `CHUTES_API_KEY`) |
| Command Code | an API key (or `COMMANDCODE_API_KEY`) |
| xAI | a pasted management key with Team ID (or `XAI_MANAGEMENT_API_KEY`) |
| OpenAI | an API key (or `OPENAI_ADMIN_KEY`) |
| Anthropic | an API key (or `ANTHROPIC_ADMIN_KEY`) |
| ai& | an API key (or `AIAND_API_KEY`) |
| Aixy | an API key with Server address (or `AIXY_API_KEY`) |
| Atlas Cloud | a pasted Atlas Cloud API key (or `ATLASCLOUD_API_KEY`) |
| Bifrost | a pasted virtual key with Server address |
| ClawRouter | an API key with Server address (or `CLAWROUTER_API_KEY`) |
| Cline | the Cline login on this computer; or a sign-in on Cline's page (Google, GitHub or email); or an API key (or `CLINE_API_KEY`, `CLINEPASS_API_KEY`) |
| Deepgram | an API key with Project ID (optional), API URL (optional) (or `DEEPGRAM_API_KEY`) |
| DeepInfra | an API key (or `DEEPINFRA_API_KEY`) |
| DevPass | a pasted DevPass API key (or `DEVPASS_API_KEY`) |
| ElevenLabs | an API key with API URL (optional) (or `ELEVENLABS_API_KEY`) |
| Fireworks | an API key with Account slug (or `FIREWORKS_API_KEY`) |
| GitKraken AI | a pasted GitKraken access token with API organization ID (or `GITKRAKEN_API_TOKEN`) |
| Helmcode | a pasted cookie header with Dashboard (default: cloud.helmcode.com, or cloud.nan.builders) |
| Hugging Face | the Hugging Face CLI login on this computer; or a pasted Hugging Face access token (or `HF_TOKEN`, `HUGGING_FACE_HUB_TOKEN`) |
| Charm Hyper | a pasted Charm Hyper API key (or `HYPER_API_KEY`) |
| LiteLLM | an API key with Server address, Show model activity (true or false) |
| llmman | an API key (optional for local daemon) with Server address |
| LLM Proxy | an API key with Server address |
| Manus | a pasted session cookie (session_id) |
| Moonshot / Kimi Open Platform | an API key (or `MOONSHOT_API_KEY`, `MOONSHOT_KEY`) |
| Muse Code | a pasted session token (or `MUSE_DEVICE_TOKEN`) |
| Neuralwatt | an API key with Server address (or `NEURALWATT_API_KEY`) |
| Nous Portal | the Hermes Agent login on this computer; or a pasted access token with Portal URL (or `NOUS_PORTAL_ACCESS_TOKEN`) |
| Perplexity | a pasted session cookie (__Secure-authjs.session-token) |
| Poe | an API key (or `POE_API_KEY`) |
| Raycast | a pasted session cookie (__raycast_session) with CSRF token (optional) |
| Replicate | a pasted session cookie (sessionid) with Account username, Organization account (true or false) |
| Sakana AI | a pasted session cookie (session) |
| sub2api | an API key with Server address |
| Synthetic | an API key (or `SYNTHETIC_API_KEY`) |
| T3 Chat | a pasted session cookie (session) |
| v0 | an API key with Scope (optional) (or `V0_API_KEY`) |
| Venice | an API key (or `VENICE_API_KEY`) |
| xKiro | an API key (or `XKIRO_API_KEY`) |
| ZenMux | a pasted management API key (or `ZENMUX_MANAGEMENT_API_KEY`) |
| Abacus AI | a pasted session cookie (sessionid) |
| Alibaba Model Studio | a pasted Coding Plan API key (or `ALIBABA_CODING_PLAN_API_KEY`, `DASHSCOPE_API_KEY`) |
| Amp | an API key (or `AMP_API_KEY`) |
| Augment | a pasted session cookie (session) |
| AWS Bedrock | the AWS CLI login on this computer; or a pasted secret access key with Access key ID, AWS region, Session token (optional) |
| Codebuff | the Codebuff login on this computer; or a GitHub sign-in; or an API key (or `CODEBUFF_API_KEY`) |
| Doubao | a pasted secret access key with Access key ID, Region (default: cn-beijing) |
| Droid | the Droid login on this computer; or an API key (or `FACTORY_API_KEY`) |
| JetBrains AI | the JetBrains IDE login on this computer |
| Kilo | the Kilo login on this computer; or a sign-in on app.kilo.ai (Google, GitHub and others); or an API key (or `KILO_API_KEY`) |
| LongCat | a pasted cookie header |
| Xiaomi MiMo | a pasted session cookie (api-platform_serviceToken) with User ID cookie (userId) |
| Mistral | a pasted cookie header (ory_session_*) with CSRF cookie (csrftoken) |
| Notion AI | a pasted session cookie (token_v2) with Workspace ID |
| Qwen Cloud | a pasted session cookie (login_aliyunid_ticket) with Security token (sec_token), CSRF cookie (login_aliyunid_csrf) |
| StepFun | a pasted session cookie (Oasis-Token) (or `STEPFUN_TOKEN`) |
| Vertex AI | the gcloud ADC login on this computer; or a pasted Google OAuth access token with Google Cloud project ID |
| Warp | an API key (or `WARP_API_KEY`, `WARP_TOKEN`) |
| Wayfinder | a pasted connection label (not sent) with Server address |
| Windsurf | the Windsurf login on this computer; or an account API key, or an Enterprise service key for the team's add-on credits (or `WINDSURF_API_KEY`) |
| ZoomMate | a pasted session bearer token (Authorization) |
| IBM Bob | an API key (or `BOBSHELL_API_KEY`) |
| Cerebras | an API key (or `CEREBRAS_API_KEY`) |
| Cloudflare Workers AI | an API token with Account ID (or `CLOUDFLARE_API_TOKEN`) |
| Novita AI | an API key (or `NOVITA_API_KEY`) |
| NanoGPT | an API key (or `NANOGPT_API_KEY`) |
| Hyperbolic | an API key (or `HYPERBOLIC_API_KEY`) |
| OpenAI-compatible relay (One API, New API) | an API key with Relay address |

Not readable yet: TypeSafe (its billing needs a page action discovered at run time), CodeRabbit (it shows usage only through its CLI).

<!-- services:end -->

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

The app Reset tab, Island and reset widgets share their presentation data in
`src/model/glanceResets.ts`. The two native surfaces also share reset cards in
`src-tauri/macos/Shared/GlanceViews.swift`: the latest reset and its author,
announcements, forecast and explanations, calendar, rhythm, statistics and history.
Changes to Reset content must update this shared contract and its parity tests.
Native reset cards follow the app's light/dark preference and blue/yellow/orange
visual hierarchy. The Island scrolls the full content; fixed-size reset widgets
use previous/next pages to keep every enabled section accessible at readable sizes.

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

Each account opens its product's official website in the app: the chat button on
its row in the Accounts screen, or **Open Claude in the App** (or ChatGPT) in its
card's menu. The popup keeps no list of sessions and the tray menu has no chat
entries. Each session opens the official website in its own native window and
persists a separate WebView profile under the app configuration directory. Closing
a window preserves its session; opening it again focuses its window or reuses its
saved profile.

`list_chat_sessions`, `create_chat_session`, and `open_chat_session` expose this
workflow to the popup. Creation persists metadata before opening the window;
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
  version opens a dialog in the popup with **Install Update**, **What's New** and **Later**, the
  tray menu offers **Install Update X…**, and while the popup is closed a system notification says
  so once per version. **Later** waits for the next check that still finds it. A failed background
  check stays silent; **Check Now** in Settings and **Check for Updates…** in the tray menu always
  report their result in the same dialog, as do the download, the install and a failed step.
- **Install Update** downloads the package that matches how the app was installed, verifies its
  signature, installs it and reopens the app, which confirms the new version with a notification
  and, the next time the popup opens, a dialog naming the version it replaced. A version installed
  outside the app (a downloaded setup, a package manager) is confirmed the same way.
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
and `SHA256SUMS`. Every release carries all three systems: publishing refuses a release that lacks
the Windows, Linux or macOS package, so installed copies on every system are offered the same
version at the same time.

1. Set the same version in `package.json`, `src-tauri/tauri.conf.json` and `Cargo.toml`
   (`[workspace.package]`), commit and push.
2. Run `node scripts/release.mjs ship`, optionally with `--notes-file notes.md`. It pushes the tag
   `v0.2.0` for the pushed commit, and the **Release** workflow checks the tag against the three
   versions, builds and signs the Windows, Linux and macOS packages from that commit and publishes
   them together as the latest release. `ship` waits for the run, then reads `latest.json` back
   from GitHub and checks every system and download link.

Nothing is published unless all three builds succeed. Pushing the tag by hand
(`git push origin HEAD:refs/tags/v0.2.0`) releases the same way without waiting. A manual run of
the workflow builds the installers without releasing them.

Without GitHub Actions the packages come from two machines. `node scripts/release.mjs local` on
Windows builds the NSIS setup there and the `.deb` and AppImage in WSL from the committed tree into
`target/release-assets/v0.2.0`; `node scripts/release.mjs mac` on a Mac builds the app bundle and the
DMG (signed with `~/.tauri/quota-control.key` or `TAURI_SIGNING_PRIVATE_KEY`) into
`target/release-assets/v0.2.0-macos`. `--publish` uploads either folder to the draft release, and
every upload merges with it: `latest.json` keeps the other platforms and `SHA256SUMS` the other
files. The machine that uploads last adds `--latest` to publish; either command stops before
building when `--latest` could not succeed because the draft lacks the other system's packages.
Publishing creates the tag, and the Release workflow then finds the release published and builds
nothing. `--skip-linux` builds only the Windows setup; such a partial build cannot be published.

### Signing key

Releases are signed with a minisign key made by `pnpm tauri signer generate`. The private key is
kept outside the repository, in `%USERPROFILE%\.tauri\quota-control.key`, where `release.mjs local`
reads it. The workflow reads it from the `TAURI_SIGNING_PRIVATE_KEY` secret:

```powershell
(Get-Content "$env:USERPROFILE\.tauri\quota-control.key" -Raw).Trim() | gh secret set TAURI_SIGNING_PRIVATE_KEY -R buidangminh23/quota-control
```

Without that secret the workflow cannot sign a release, and `ship` stops before pushing the tag.

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
