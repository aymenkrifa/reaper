#!/bin/sh
# reaper installer — https://reaper.aymenkrifa.com
#   curl -LsSf https://reaper.aymenkrifa.com/install.sh | sh
#
# Downloads the static musl binary for this machine from the latest GitHub
# release, verifies it against the published sha256, and drops it in ~/.local/bin.
# Override the destination with REAPER_BIN_DIR=/somewhere.
set -eu

REPO="aymenkrifa/reaper"
BASE="https://github.com/$REPO/releases/latest/download"

# Where to put the binary: an explicit override wins; root gets a system dir
# that's already on PATH (so 'reaper' just works, no profile edits); everyone
# else gets a no-sudo user dir.
if [ -n "${REAPER_BIN_DIR:-}" ]; then
  BIN_DIR="$REAPER_BIN_DIR"
elif [ "$(id -u)" = 0 ]; then
  BIN_DIR="/usr/local/bin"
else
  BIN_DIR="$HOME/.local/bin"
fi

say() { printf '%s\n' "reaper: $*"; }
die() { printf '%s\n' "reaper: $*" >&2; exit 1; }

[ "$(uname -s)" = "Linux" ] || die "reaper is Linux-only (it reads /proc)."
case "$(uname -m)" in
  x86_64 | amd64)  ARCH=x86_64 ;;
  aarch64 | arm64) ARCH=aarch64 ;;
  *) die "unsupported architecture: $(uname -m)" ;;
esac

command -v curl >/dev/null 2>&1 || die "curl is required."
command -v tar  >/dev/null 2>&1 || die "tar is required."

TARBALL="reaper-$ARCH-unknown-linux-musl.tar.gz"
SUM="reaper-$ARCH-unknown-linux-musl.sha256" # taiki-e drops .tar.gz before .sha256
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
fetch() { curl --proto '=https' --tlsv1.2 -fsSL "$1" -o "$2"; }

say "downloading $TARBALL…"
fetch "$BASE/$TARBALL" "$TMP/$TARBALL"
fetch "$BASE/$SUM" "$TMP/$SUM"

say "verifying checksum…"
expected="$(awk 'NR==1{print $1}' "$TMP/$SUM")"
[ -n "$expected" ] || die "could not read the published checksum."
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$TMP/$TARBALL" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  actual="$(shasum -a 256 "$TMP/$TARBALL" | awk '{print $1}')"
else
  die "need sha256sum or shasum to verify the download."
fi
[ "$expected" = "$actual" ] || die "checksum mismatch — refusing to install."

tar -xzf "$TMP/$TARBALL" -C "$TMP"
[ -f "$TMP/reaper" ] || die "archive did not contain the reaper binary."
mkdir -p "$BIN_DIR"
install -m 755 "$TMP/reaper" "$BIN_DIR/reaper"
say "installed → $BIN_DIR/reaper"

case ":$PATH:" in
  *":$BIN_DIR:"*)
    say "done — run 'reaper' (or 'sudo reaper' to include other users' ports)."
    ;;
  *)
    say "$BIN_DIR isn't on your PATH yet."
    say "this session:  export PATH=\"$BIN_DIR:\$PATH\""
    say "to keep it, append that line to ~/.bashrc (bash) or ~/.zshrc (zsh), then run 'reaper'."
    ;;
esac
