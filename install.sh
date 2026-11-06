#!/bin/sh
# Install streamprobe from the GitHub releases of Hippynet/stream-quality-probe.
#
#   curl -fsSL https://raw.githubusercontent.com/Hippynet/stream-quality-probe/main/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/Hippynet/stream-quality-probe/main/install.sh | sh -s 0.1.0.1
#   sh install.sh [VERSION]
#
# Downloads the build for this machine (macOS universal, Linux x86_64 or
# Linux arm64), checks it against the release's SHA256SUMS and installs it to
# $DEST, or else the first writable of /opt/homebrew/bin, /usr/local/bin and
# ~/.local/bin. Needs curl or wget, tar, and sha256sum or shasum. No GitHub
# account or token is needed.
#
# streamprobe needs ffmpeg at runtime; this script tells you how to get it
# if it's missing.
set -eu

REPO=Hippynet/stream-quality-probe
BASE=https://github.com/$REPO

say() { printf '%s\n' "$*"; }
die() {
  printf 'streamprobe install: %s\n' "$*" >&2
  exit 1
}
have() { command -v "$1" >/dev/null 2>&1; }

# fetch URL FILE
fetch() {
  if have curl; then
    curl -fsSL --proto '=https' --tlsv1.2 --retry 3 -o "$2" "$1"
  elif have wget; then
    wget -q -O "$2" "$1"
  else
    die "needs curl or wget"
  fi
}

# The tag that /releases/latest redirects to (no API call, so no rate limit).
latest_version() {
  if have curl; then
    url=$(curl -fsSLI --proto '=https' --tlsv1.2 --retry 3 -o /dev/null -w '%{url_effective}' "$BASE/releases/latest") ||
      die "could not reach $BASE/releases/latest"
  elif have wget; then
    url=$(wget -S --spider "$BASE/releases/latest" 2>&1 | sed -n 's/^ *[Ll]ocation: *\([^ ]*\).*/\1/p' | tail -n 1 | tr -d '\r')
  else
    die "needs curl or wget"
  fi
  case $url in
    */releases/tag/*) printf '%s\n' "${url##*/releases/tag/}" ;;
    *) die "no release found at $BASE/releases (got: ${url:-nothing})" ;;
  esac
}

valid_version() {
  printf '%s\n' "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$'
}

sha256_of() {
  if have sha256sum; then
    sha256sum "$1" | awk '{print $1}'
  elif have shasum; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    die "needs sha256sum or shasum to verify the download"
  fi
}

ffmpeg_hint() {
  have ffmpeg && return 0
  say ""
  say "streamprobe needs ffmpeg, which isn't on your PATH yet. To install it:"
  os_ids=""
  if [ "$(uname -s)" = Darwin ]; then
    os_ids=macos
  elif [ -r /etc/os-release ]; then
    os_ids=$(sed -n 's/^ID\(_LIKE\)\{0,1\}=//p' /etc/os-release | tr -d '"' | tr '\n' ' ')
  fi
  case " $os_ids " in
    *" macos "*) say "  brew install ffmpeg" ;;
    *" debian "* | *" ubuntu "*) say "  sudo apt install ffmpeg" ;;
    *" rhel "* | *" centos "* | *" rocky "* | *" almalinux "*)
      say "  enable EPEL and RPM Fusion (https://rpmfusion.org/Configuration), then:"
      say "  sudo dnf install ffmpeg"
      ;;
    *" fedora "*)
      say "  enable RPM Fusion (https://rpmfusion.org/Configuration), then:"
      say "  sudo dnf install ffmpeg"
      ;;
    *)
      say "  macOS:            brew install ffmpeg"
      say "  Debian/Ubuntu:    sudo apt install ffmpeg"
      say "  RHEL/Alma/Rocky:  enable EPEL and RPM Fusion (https://rpmfusion.org/Configuration),"
      say "                    then sudo dnf install ffmpeg"
      ;;
  esac
}

main() {
  case "${1:-}" in
    -h | --help)
      say "usage: install.sh [VERSION]   (e.g. 0.1.0.0; default: the latest release)"
      say "Set DEST to choose the install directory."
      return 0
      ;;
  esac

  os=$(uname -s)
  arch=$(uname -m)
  case "$os/$arch" in
    Darwin/*) asset=macos-universal ;;
    Linux/x86_64) asset=linux-x86_64 ;;
    Linux/aarch64 | Linux/arm64) asset=linux-arm64 ;;
    *) die "there is no prebuilt binary for $os/$arch. Build from source instead: cargo install --locked --git $BASE" ;;
  esac

  version=${1:-}
  if [ -z "$version" ]; then
    version=$(latest_version)
  fi
  valid_version "$version" || die "not a streamprobe version: '$version' (expected e.g. 0.1.0.0)"

  tmp=$(mktemp -d 2>/dev/null || mktemp -d -t streamprobe) || die "could not create a temporary directory"
  [ -d "$tmp" ] || die "could not create a temporary directory"
  trap 'rm -rf "$tmp"' EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM

  archive="streamprobe-$version-$asset.tar.gz"
  say "Downloading streamprobe $version ($asset)..."
  fetch "$BASE/releases/download/$version/$archive" "$tmp/$archive" ||
    die "could not download $archive from $BASE/releases/tag/$version"
  fetch "$BASE/releases/download/$version/SHA256SUMS" "$tmp/SHA256SUMS" ||
    die "could not download SHA256SUMS for $version"

  expected=$(awk -v f="$archive" '$2 == f || $2 == "*" f { print $1 }' "$tmp/SHA256SUMS")
  printf '%s\n' "$expected" | grep -Eq '^[0-9a-f]{64}$' || die "SHA256SUMS has no single entry for $archive"
  actual=$(sha256_of "$tmp/$archive")
  [ "$actual" = "$expected" ] || die "checksum mismatch for $archive (expected $expected, got $actual)"
  say "Checksum OK."

  tar -xzf "$tmp/$archive" -C "$tmp" streamprobe || die "could not unpack $archive"

  if [ -z "${DEST:-}" ]; then
    for d in /opt/homebrew/bin /usr/local/bin "$HOME/.local/bin"; do
      if [ -d "$d" ] && [ -w "$d" ]; then
        DEST=$d
        break
      fi
    done
    if [ -z "${DEST:-}" ]; then
      DEST=$HOME/.local/bin
      mkdir -p "$DEST" || die "could not create $DEST"
    fi
  fi
  if [ ! -d "$DEST" ] || [ ! -w "$DEST" ]; then
    die "can't write to $DEST (set DEST to a directory you can write to)"
  fi

  # Copy then rename, so a running streamprobe is replaced cleanly.
  cp "$tmp/streamprobe" "$DEST/.streamprobe.new.$$"
  chmod 0755 "$DEST/.streamprobe.new.$$"
  mv -f "$DEST/.streamprobe.new.$$" "$DEST/streamprobe"
  # The macOS build is ad-hoc signed, not notarised: clear the quarantine flag if set.
  if [ "$os" = Darwin ] && have xattr; then
    xattr -d com.apple.quarantine "$DEST/streamprobe" 2>/dev/null || true
  fi

  installed=$("$DEST/streamprobe" --version 2>&1) || die "installed to $DEST/streamprobe, but it doesn't run: $installed"
  say "Installed $installed to $DEST/streamprobe"
  case ":$PATH:" in
    *":$DEST:"*) ;;
    *) say "Note: $DEST is not on your PATH. Add it, e.g.: export PATH=\"$DEST:\$PATH\"" ;;
  esac
  ffmpeg_hint
}

# Everything runs from here, so a partly downloaded script does nothing.
main "$@"
