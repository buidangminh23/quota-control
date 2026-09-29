# Security Policy

Quota Control shows the limits of your AI coding subscriptions, so it handles account sign-ins and
tokens on your computer. This page says which versions receive fixes, how to report a vulnerability
privately, and what the app is designed to protect.

## Supported versions

Only the latest release receives security fixes. There are no long-term branches: a fix ships as a
new release, which installed copies offer under **Settings → App Updates**, and which the
[Releases](https://github.com/buidangminh23/quota-control/releases/latest) page also provides.

| Version | Supported |
| --- | --- |
| [Latest release](https://github.com/buidangminh23/quota-control/releases/latest) | ✅ |
| Every older release | ❌ Update to the latest release first |

## Reporting a vulnerability

Report vulnerabilities privately through GitHub: open this repository's **Security** tab and choose
**Report a vulnerability**, or go straight to the
[private report form](https://github.com/buidangminh23/quota-control/security/advisories/new).
Please do not open a public issue, pull request or discussion for a vulnerability.

A useful report includes:

- the Quota Control version (**Settings → App Updates**) and your operating system;
- the steps to reproduce it and what an attacker gains;
- a proof of concept, if you have one, with real tokens, cookies, email addresses and other account
  data removed.

Quota Control is maintained by one person, so there is no fixed response time. Replies come in the
private advisory thread. A confirmed issue is fixed in a new release, after which the advisory is
published, crediting you unless you ask not to be named.

## Scope

Reports about this repository's code are welcome, for example:

- stored credentials (accounts signed in through the browser, and the Claude Code and Codex CLI
  logins the app reads) reaching another user, another program, the popup, logs, the local HTTP API
  or `usagectl` output;
- a chat window, a web page or another program making the app run its native commands;
- the app installing an update that the release key did not sign, or an older signed version
  presented as a newer one;
- the local HTTP API accepting connections from other computers;
- files the app installs (the `usagectl` copy and its PATH entry, the data folders) being writable
  by other users, or the app replacing a file it did not create.

Out of scope:

- vulnerabilities in the providers' own services (Claude, ChatGPT and Codex, Google sign-in and the
  others); please report those to the provider;
- attacks that need code already running as your user account, which can read everything that
  account can read, including the CLIs' own credential files;
- the behaviour listed under **Known limitations** below.

## How the app protects your data

- **Credentials** are protected with per-user DPAPI on Windows and with owner-only folders and files
  (0700/0600) on Linux, and they are never returned to the popup through IPC. The Claude Code and
  Codex CLI logins are read at each refresh; the app never copies them into its own account store
  and never renews them.
- **Browser sign-in** opens the provider's page in your browser, which returns to a listener on this
  computer's loopback address.
- **Native commands** can only be called by the bundled popup. Chat windows and their sign-in popups
  cannot call them, and chat windows cannot navigate to local or native addresses.
- **Network requests** come from the Rust core. The popup's web view cannot reach the network: its
  Content Security Policy only allows the app's IPC. The core talks to each provider's own usage and
  status endpoints, GitHub Releases for updates, Vietcombank for the exchange rate, and the public
  benchmark and reset sources shown in the app. The chat windows load the official Claude and
  ChatGPT websites. There is no analytics or telemetry.
- **Local history**, meaning the token ledger and the model-quality counts, is built from the Claude
  Code and Codex logs on this computer and stays in the app's data folder.
- **Updates** must carry a minisign signature from the release key whose public half is built into
  the app (`plugins.updater.pubkey` in `src-tauri/tauri.conf.json`), and the signature must name the
  version being installed. Each release also lists its checksums in `SHA256SUMS`.
- **Dependencies** come from crates.io and npm at the versions the lock files pin. When a fix has
  not reached a version the app can use, it is backported into a copy under `third_party/`: the
  Linux build compiles glib 0.18.5 with the fix for
  [GHSA-wrw7-89jp-8q8g](https://github.com/advisories/GHSA-wrw7-89jp-8q8g), because Tauri 2's GTK
  stack cannot use glib 0.20.

## Known limitations

- The local HTTP API on `http://127.0.0.1:6736` allows any origin (CORS `*`), as OpenUsage does, so
  any program or web page on this computer can read your limits, usage, plans and card names while
  the app runs. Card names can include an account's email address. The API never returns tokens.
- The Windows setup is not code-signed, so SmartScreen warns the first time it runs. Check a
  downloaded setup against `SHA256SUMS` on its release page; updates installed from inside the app
  are verified by their minisign signature.
- Each embedded chat session keeps the website's own cookies in a separate WebView profile inside
  the app's data folder, as a browser profile does.
