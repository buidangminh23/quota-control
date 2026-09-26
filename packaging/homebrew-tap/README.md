# buidangminh23/tap

Homebrew casks for apps published by [buidangminh23](https://github.com/buidangminh23).

| Cask | App |
|---|---|
| `quota-control` | [Quota Control](https://github.com/buidangminh23/quota-control): Claude, Codex, Cursor and other AI coding limits in the menu bar, a Dynamic Island around the notch and a desktop widget |

## Install

```bash
brew tap buidangminh23/tap
brew trust --tap buidangminh23/tap
brew install --cask quota-control
```

Homebrew 6 and later load casks from other taps only after `brew trust`. Quota Control needs an
Apple Silicon Mac with macOS 14 (Sonoma) or later.

The app is signed ad hoc, not notarized by Apple. The cask clears the download's quarantine flag
after installing, so the app opens without the **Open Anyway** step in System Settings that a
browser download needs, and registers the desktop widget with the widget gallery.

Once installed, Quota Control updates itself from its GitHub releases, so `brew upgrade` leaves it
alone (`auto_updates true`). `brew upgrade --greedy` moves it to the version the cask names.

## Uninstall

```bash
brew uninstall --cask quota-control
```

This quits the app and removes it with its `usagectl` command. Accounts and settings stay in
`~/Library/Application Support/usage-control`; `brew uninstall --zap --cask quota-control` removes
them too, along with the login item.

The cask links `usagectl` into Homebrew's `bin`, so the command works in any shell that has
Homebrew on its `PATH`.

## How the cask follows releases

`.github/workflows/update-casks.yml` runs every three hours and on demand. It reads the latest
Quota Control release, takes the DMG's checksum from the release's `SHA256SUMS`, confirms it against
the downloaded DMG, and commits the new version and checksum. A release without a macOS DMG leaves
the cask where it is. To follow a release at once:

```bash
gh workflow run update-casks.yml -R buidangminh23/homebrew-tap
```

GitHub turns off scheduled workflows in a repository with no activity for 60 days. If that
happens, turn it back on with `gh workflow enable update-casks.yml -R buidangminh23/homebrew-tap`.
