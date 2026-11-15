# @minhspark/quota-control

Installer for [Quota Control](https://github.com/buidangminh23/quota-control): AI usage limits, reset countdowns and token history in your Windows/Linux system tray or macOS menu bar.

```bash
npx @minhspark/quota-control
```

It downloads the latest release for your computer, checks it against the release's `SHA256SUMS` and installs it:

- **Windows x64:** runs the setup program.
- **Ubuntu / Debian x64:** installs the `.deb` with `sudo apt-get install`.
- **Other Linux x64:** saves the AppImage as `~/.local/bin/quota-control.AppImage`.
- **macOS (Apple Silicon):** runs the project's `install-macos.sh`, which verifies the app and installs it into Applications.

Use `npx @minhspark/quota-control --download <folder>` to only download and verify the installer.

MIT License.
