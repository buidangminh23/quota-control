#!/bin/bash
# Installs or updates Quota Control on an Apple Silicon Mac from the GitHub releases:
#
#   curl -fsSL https://raw.githubusercontent.com/buidangminh23/quota-control/main/scripts/install-macos.sh | bash
#
# It downloads the release's app archive, checks it against the release's SHA256SUMS, puts
# `Quota Control.app` in /Applications (~/Applications when /Applications is not writable), registers
# it so its desktop widgets appear in the widget gallery, and opens it. The app is not notarized;
# files curl downloads carry no quarantine flag, so Gatekeeper lets it open without the
# "Open Anyway" detour a browser download needs. Once installed, the app updates itself.
#
# Options: --version X.Y.Z (default: the latest release), --dir FOLDER, --no-launch, --uninstall.
# Uninstalling moves the app to the Trash and removes the usagectl link and the login item; accounts
# and settings in ~/Library/Application Support/usage-control stay.
#
# Everything runs from `main`, so a download cut short installs nothing; the new copy is staged
# beside the old one before the running app is closed, and swapped in with two renames.
set -euo pipefail

REPO="buidangminh23/quota-control"
APP_NAME="Quota Control.app"
EXECUTABLE="quota-control"
LOGIN_ITEM="$HOME/Library/LaunchAgents/Usage Control.plist"
MINIMUM_MACOS=14
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"

work=""

say() { printf '%s\n' "$*"; }
die() { printf 'install-macos: %s\n' "$*" >&2; exit 1; }

installed_app() {
  local folder
  for folder in "$1" /Applications "$HOME/Applications"; do
    [ -n "$folder" ] && [ -d "$folder/$APP_NAME" ] && { printf '%s\n' "$folder/$APP_NAME"; return 0; }
  done
  return 1
}

stop_app() {
  if pgrep -x "$EXECUTABLE" >/dev/null 2>&1; then
    say "Closing the running Quota Control…"
    pkill -TERM -x "$EXECUTABLE" || true
    for _ in 1 2 3 4 5 6 7 8 9 10; do
      pgrep -x "$EXECUTABLE" >/dev/null 2>&1 || return 0
      sleep 0.5
    done
    pkill -KILL -x "$EXECUTABLE" || true
  fi
}

remove_cli_link() {
  local link="$HOME/.local/bin/usagectl"
  if [ -L "$link" ] && [[ "$(readlink "$link")" == *"/$APP_NAME/Contents/MacOS/usagectl" ]]; then
    rm -f "$link"
  fi
}

remove_login_item() {
  if [ -f "$LOGIN_ITEM" ] && grep -q "/$APP_NAME/Contents/MacOS/" "$LOGIN_ITEM"; then
    launchctl bootout "gui/$(id -u)" "$LOGIN_ITEM" >/dev/null 2>&1 || true
    rm -f "$LOGIN_ITEM"
  fi
}

uninstall() {
  local app
  app="$(installed_app "$1")" || die "Quota Control is not installed"
  stop_app
  "$LSREGISTER" -u "$app" >/dev/null 2>&1 || true
  if command -v trash >/dev/null 2>&1; then
    trash "$app"
  else
    rm -rf "$app"
  fi
  remove_cli_link
  remove_login_item
  say "Removed $app. Accounts and settings stay in ~/Library/Application Support/usage-control."
}

main() {
  local version="" target_dir="" launch=1 remove=0
  while [ $# -gt 0 ]; do
    case "$1" in
      --version) version="${2:-}"; shift 2 ;;
      --dir) target_dir="${2:-}"; shift 2 ;;
      --no-launch) launch=0; shift ;;
      --uninstall) remove=1; shift ;;
      -h|--help) say "usage: install-macos.sh [--version X.Y.Z] [--dir FOLDER] [--no-launch] [--uninstall]"; exit 0 ;;
      *) die "unknown option $1" ;;
    esac
  done
  version="${version#v}"

  [ "$(uname -s)" = "Darwin" ] || die "this installer is for macOS"
  [ "$(uname -m)" = "arm64" ] || die "Quota Control for macOS runs on Apple Silicon Macs only"
  local major
  major="$(sw_vers -productVersion | cut -d. -f1)"
  [ "$major" -ge "$MINIMUM_MACOS" ] || die "Quota Control needs macOS $MINIMUM_MACOS or later"

  if [ "$remove" -eq 1 ]; then
    uninstall "$target_dir"
    return 0
  fi

  if [ -z "$target_dir" ]; then
    local existing
    if existing="$(installed_app "")"; then
      target_dir="$(dirname "$existing")"
    elif [ -w /Applications ]; then
      target_dir="/Applications"
    else
      target_dir="$HOME/Applications"
    fi
  fi
  mkdir -p "$target_dir" 2>/dev/null || die "cannot create $target_dir"
  [ -w "$target_dir" ] || die "$target_dir is not writable; run again with --dir \"$HOME/Applications\""

  work="$(mktemp -d "${TMPDIR:-/tmp}/quota-control.XXXXXX")"
  trap 'rm -rf "$work"' EXIT

  if [ -z "$version" ]; then
    curl -fsSL "https://github.com/$REPO/releases/latest/download/latest.json" -o "$work/latest.json" \
      || die "cannot read the latest release"
    version="$(/usr/bin/plutil -extract version raw -o - "$work/latest.json" 2>/dev/null)" \
      || die "the latest release has no version"
  fi
  local base="https://github.com/$REPO/releases/download/v$version"
  local archive="Quota-Control_${version}_aarch64.app.tar.gz"

  say "Downloading Quota Control $version…"
  curl -fL --progress-bar "$base/$archive" -o "$work/$archive" || die "release v$version has no $archive"
  curl -fsSL "$base/SHA256SUMS" -o "$work/SHA256SUMS" || die "release v$version has no SHA256SUMS"
  local expected actual
  expected="$(awk -v name="$archive" '$2 == name || $2 == "*" name { print $1 }' "$work/SHA256SUMS")"
  [ -n "$expected" ] || die "SHA256SUMS does not list $archive"
  actual="$(shasum -a 256 "$work/$archive" | awk '{ print $1 }')"
  [ "$expected" = "$actual" ] || die "checksum mismatch for $archive; nothing was installed"

  mkdir -p "$work/app"
  tar -xzf "$work/$archive" -C "$work/app"
  [ -d "$work/app/$APP_NAME" ] || die "the archive does not contain $APP_NAME"
  xattr -dr com.apple.quarantine "$work/app/$APP_NAME" 2>/dev/null || true
  codesign --verify --deep --strict "$work/app/$APP_NAME" >/dev/null 2>&1 || die "the app's signature does not verify"

  local destination="$target_dir/$APP_NAME"
  local staged="$target_dir/.$APP_NAME.installing"
  local previous="$target_dir/.$APP_NAME.previous"
  rm -rf "$staged" "$previous"
  ditto "$work/app/$APP_NAME" "$staged" || { rm -rf "$staged"; die "cannot copy the app into $target_dir"; }

  stop_app
  if [ -d "$destination" ]; then
    mv "$destination" "$previous" || { rm -rf "$staged"; die "cannot replace $destination"; }
  fi
  if ! mv "$staged" "$destination"; then
    [ -d "$previous" ] && mv "$previous" "$destination"
    rm -rf "$staged"
    die "cannot move the new app into place; the previous version was kept"
  fi
  rm -rf "$previous"
  "$LSREGISTER" -f "$destination" >/dev/null 2>&1 || true
  pluginkit -a "$destination/Contents/PlugIns/QuotaControlWidget.appex" >/dev/null 2>&1 || true

  say "Installed Quota Control $version in $destination."
  say "Add a widget: right-click the desktop, choose Edit Widgets and search for Quota Control."
  if [ "$launch" -eq 1 ]; then
    open "$destination"
  fi
}

main "$@"
