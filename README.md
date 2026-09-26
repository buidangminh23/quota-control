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

## Upgrade compatibility

Quota Control retains the previous application storage directories, credential protection,
WebView profiles and application identifier so existing accounts and settings remain available.
The `usagectl` integration command also remains supported. Legacy names in these stable
identifiers are intentional; the application and build executable are named Quota Control
and `quota-control` respectively.

## License

[MIT](LICENSE). Original work copyright Robin Ebers; port copyright Bui Dang Minh.
