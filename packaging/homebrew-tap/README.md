# Homebrew cask

The Quota Control cask lives in the published tap
[buidangminh23/homebrew-tap](https://github.com/buidangminh23/homebrew-tap):

```sh
brew install --cask buidangminh23/tap/quota-control
```

The tap's `Casks/quota-control.rb`, `scripts/update-quota-control.sh` and
`.github/workflows/update-casks.yml` are the only copies. The workflow reads the latest release,
takes the DMG's checksum from `SHA256SUMS`, confirms it against the downloaded DMG, then audits,
installs and checks the cask on macOS before committing. A release without a macOS DMG leaves the
cask where it was.

Point the cask at a release at once from the tap:

```sh
gh workflow run update-casks.yml -R buidangminh23/homebrew-tap
gh workflow run update-casks.yml -R buidangminh23/homebrew-tap -f tag=vX.Y.Z
```

Files the app writes and the cask's uninstall and zap steps must stay in step. When the app adds a
path under `~/Library` or changes its bundle identifiers, update the cask's `zap` list in the tap.
