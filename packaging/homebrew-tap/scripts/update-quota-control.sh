#!/bin/bash
# Points Casks/quota-control.rb at the newest Quota Control release that ships a macOS DMG.
#
#   GH_TOKEN=... scripts/update-quota-control.sh [--tag vX.Y.Z]
#
# The checksum is taken from the release's SHA256SUMS and confirmed against the downloaded DMG, so
# a cask never carries a checksum the asset does not match. A release without a DMG (a Windows or
# Linux-only build) leaves the cask where it is. Prints the version the cask ends on.
set -euo pipefail

REPO="buidangminh23/quota-control"
CASK="Casks/quota-control.rb"

die() { printf 'update-quota-control: %s\n' "$*" >&2; exit 1; }

tag=""
while [ $# -gt 0 ]; do
  case "$1" in
    --tag) tag="${2:-}"; shift 2 ;;
    *) die "unknown option $1" ;;
  esac
done

[ -f "$CASK" ] || die "run from the tap's root; $CASK is missing"
current="$(sed -n 's/^  version "\(.*\)"$/\1/p' "$CASK")"
[ -n "$current" ] || die "$CASK has no version line"

if [ -z "$tag" ]; then
  tag="$(gh release view --repo "$REPO" --json tagName --jq .tagName)"
fi
version="${tag#v}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "unexpected release tag $tag"

dmg="Quota-Control_${version}_aarch64.dmg"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

gh release download "$tag" --repo "$REPO" --pattern SHA256SUMS --dir "$work" \
  || die "release $tag has no SHA256SUMS"
listed="$(awk -v name="$dmg" '$2 == name || $2 == "*" name { print $1 }' "$work/SHA256SUMS")"
if [ -z "$listed" ]; then
  printf 'Release %s ships no %s; the cask stays at %s.\n' "$tag" "$dmg" "$current" >&2
  printf '%s\n' "$current"
  exit 0
fi
[[ "$listed" =~ ^[0-9a-f]{64}$ ]] || die "SHA256SUMS lists a malformed checksum for $dmg"

gh release download "$tag" --repo "$REPO" --pattern "$dmg" --dir "$work" \
  || die "release $tag lists $dmg but does not carry it"
actual="$(shasum -a 256 "$work/$dmg" | awk '{ print $1 }')"
[ "$actual" = "$listed" ] || die "$dmg does not match its SHA256SUMS entry"

sed -i.bak \
  -e "s/^  version \".*\"$/  version \"$version\"/" \
  -e "s/^  sha256 \".*\"$/  sha256 \"$actual\"/" \
  "$CASK"
rm -f "$CASK.bak"
grep -q "^  version \"$version\"$" "$CASK" || die "could not write the version into $CASK"
grep -q "^  sha256 \"$actual\"$" "$CASK" || die "could not write the checksum into $CASK"
printf '%s\n' "$version"
