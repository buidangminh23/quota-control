<p align="center"><img src="src-tauri/icons/128x128@2x.png" width="96" height="96" alt="Quota Control app icon"></p>

# Quota Control

AI usage limits, reset countdowns and token history in your Windows/Linux system tray or macOS menu bar.

## Install

Every release ships Windows, macOS and Linux builds from the same commit: see [Releases](https://github.com/buidangminh23/quota-control/releases/latest).
The recommended PowerShell, install-script and `curl` commands check the download against the release's `SHA256SUMS` before installing.

### Windows (x64)

**PowerShell** (recommended):

```powershell
$ErrorActionPreference = 'Stop'; $ProgressPreference = 'SilentlyContinue'
$release = Invoke-RestMethod 'https://api.github.com/repos/buidangminh23/quota-control/releases/latest'
$asset = $release.assets | Where-Object name -Like '*_x64-setup.exe'
$sums = $release.assets | Where-Object name -EQ 'SHA256SUMS'
$installer = Join-Path $env:TEMP $asset.name
Invoke-WebRequest $asset.browser_download_url -OutFile $installer
Invoke-WebRequest $sums.browser_download_url -OutFile "$env:TEMP\SHA256SUMS"
$expected = (Select-String -Path "$env:TEMP\SHA256SUMS" -SimpleMatch $asset.name).Line.Split(' ')[0]
if ((Get-FileHash $installer -Algorithm SHA256).Hash -ne $expected) { throw 'Checksum mismatch' }
Start-Process $installer
```

**GitHub CLI:**

```powershell
gh release download --repo buidangminh23/quota-control --pattern '*_x64-setup.exe' --dir $env:TEMP --clobber
Start-Process (Get-ChildItem "$env:TEMP\Quota-Control_*_x64-setup.exe" | Sort-Object LastWriteTime | Select-Object -Last 1).FullName
```

**Uninstall:**

```powershell
& "$env:LOCALAPPDATA\Quota Control\uninstall.exe"
```

Installs for the current user in `%LOCALAPPDATA%\Quota Control`. The installer is not code-signed; Windows SmartScreen may ask for confirmation.

### macOS (Apple Silicon, macOS 14+)

**Install script** (recommended):

```sh
curl -fsSL https://raw.githubusercontent.com/buidangminh23/quota-control/main/scripts/install-macos.sh | bash
```

Installs `Quota Control.app` in `/Applications` (`~/Applications` if that is not writable), links `usagectl` and opens the app.
Files downloaded this way carry no quarantine flag, so the app opens without the **Open Anyway** step.

**A specific version:**

```sh
curl -fsSL https://raw.githubusercontent.com/buidangminh23/quota-control/main/scripts/install-macos.sh | bash -s -- --version 0.3.18
```

**Disk image with GitHub CLI:**

```sh
gh release download --repo buidangminh23/quota-control --pattern '*_aarch64.dmg' --output QuotaControl.dmg --clobber &&
open QuotaControl.dmg
```

Drag **Quota Control** to **Applications**. The app is not notarized by Apple; a copy downloaded in a browser may need
**System Settings → Privacy & Security → Open Anyway**.

**Uninstall** (accounts and settings stay):

```sh
curl -fsSL https://raw.githubusercontent.com/buidangminh23/quota-control/main/scripts/install-macos.sh | bash -s -- --uninstall
```

### Linux (x64)

**Ubuntu/Debian — `.deb`:**

```sh
url=$(curl -fsSL https://api.github.com/repos/buidangminh23/quota-control/releases/latest | grep -o 'https://[^"]*_amd64\.deb' | head -n1) &&
curl -fLO "$url" &&
curl -fsSL "${url%/*}/SHA256SUMS" | sha256sum --check --ignore-missing &&
sudo apt install "./${url##*/}"
```

**Other distributions — AppImage:**

```sh
url=$(curl -fsSL https://api.github.com/repos/buidangminh23/quota-control/releases/latest | grep -o 'https://[^"]*_amd64\.AppImage' | head -n1) &&
curl -fLO "$url" &&
curl -fsSL "${url%/*}/SHA256SUMS" | sha256sum --check --ignore-missing &&
chmod +x "${url##*/}" &&
"./${url##*/}"
```

If FUSE is unavailable, start it with `APPIMAGE_EXTRACT_AND_RUN=1` in front.

**GitHub CLI:**

```sh
gh release download --repo buidangminh23/quota-control --pattern '*_amd64.deb' --output quota-control.deb --clobber &&
sudo apt install ./quota-control.deb
```

```sh
gh release download --repo buidangminh23/quota-control --pattern '*_amd64.AppImage' --output quota-control.AppImage --clobber &&
chmod +x quota-control.AppImage &&
./quota-control.AppImage
```

**Uninstall:** `sudo apt remove quota-control` for the `.deb`; delete the file for the AppImage.

A desktop with system tray support is recommended (on GNOME, the AppIndicator extension).

### Updates

After the first install the app updates itself: new versions install while the popup is closed, verified with the
release signing key. **Settings → App Updates** turns that off, and **Check Now** looks for one right away.

## Preview

https://github.com/user-attachments/assets/05afff30-24bb-41df-a1bd-a7db58bdaac7

**Usage at a glance in the system tray**

![AI account usage in the Quota Control system tray](assets/readme/tray-usage.png)

<details>
<summary>Reset dashboard</summary>

<p><img src="assets/readme/reset-dashboard.png" width="308" alt="Quota Control reset dashboard with the latest reset, forecast and history"></p>

</details>

## Features

- Claude, Codex, Cursor, Copilot and many other AI services in one dashboard.
- Account limits, credits and reset countdowns, depending on what each service exposes.
- Local token history and estimated API-equivalent costs, separate from subscription charges.
- Quota notifications, a global shortcut and automatic updates.
- Dynamic Island and desktop widgets on macOS.

## Get started

1. Open Quota Control from the system tray or menu bar.
2. Existing Claude Code and Codex CLI logins appear automatically. Use **Accounts → +** to connect other accounts with the methods offered for each service.
3. Choose the accounts and readings to display in **Customize** and **Settings**.

The backend runs locally; provider requests go to the relevant services. Local token history covers this computer only.

## Command line and API

`usagectl` is installed with the app. Open a new terminal after the first launch:

```sh
usagectl
usagectl claude
usagectl codex --force
```

The CLI returns JSON and works with the app closed. While the app runs, the local API is available at
`http://127.0.0.1:6736/v1/limits`. It allows requests from any origin, so local programs and web pages can read these limits.

## Development

Requires Rust, Node.js, pnpm and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
pnpm install
pnpm tauri dev
```

Validate with `cargo test --workspace`, `pnpm test` and `pnpm build`. Build an installer on its target OS with `pnpm tauri build`.
See [scanner accounting](crates/uc-logscan/README.md) and [pricing provenance](crates/uc-pricing/README.md) for token and cost details.

## License and credits

[MIT](LICENSE). Based on [OpenUsage](https://github.com/robinebers/openusage) by Robin Ebers; port by Bui Dang Minh.
Quota Control is an independent, unofficial port and is not affiliated with or endorsed by OpenUsage's author.
