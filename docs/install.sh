#!/bin/sh
# reaper installer — https://reaper.aymenkrifa.com
#   curl -LsSf https://reaper.aymenkrifa.com/install.sh | sh
#
# Downloads the static musl binary for this machine from the latest GitHub
# release, verifies it against the published sha256, and drops it in ~/.local/bin.
# Override the destination with REAPER_BIN_DIR=/somewhere, or silence the
# progress lines with REAPER_QUIET=1 (errors still print).
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

# Readable output: colour only on a real terminal (honours NO_COLOR), plain
# when piped to a file. REAPER_QUIET=1 hushes everything but errors, which
# always go to stderr.
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  b=$(printf '\033[1m');   dim=$(printf '\033[2m'); grn=$(printf '\033[32m')
  ylw=$(printf '\033[33m'); red=$(printf '\033[31m'); rst=$(printf '\033[0m')
else
  b=; dim=; grn=; ylw=; red=; rst=
fi
quiet() { [ -n "${REAPER_QUIET:-}" ]; }
line() { quiet || printf '%s\n' "$*"; }
ok()   { quiet || printf '  %s✓%s %-10s %s\n' "$grn" "$rst" "$1" "$2"; }
warn() { quiet || printf '  %s!%s %s\n' "$ylw" "$rst" "$*"; }
die()  { printf '\n  %serror%s %s\n' "$red" "$rst" "$*" >&2; exit 1; }

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

line ""
line "  ${b}reaper${rst}${dim} · a linux tui for listing & killing listening ports${rst}"
line ""
ok "target" "$ARCH-unknown-linux-musl"

fetch "$BASE/$TARBALL" "$TMP/$TARBALL" || die "could not download $TARBALL — is there a release for your platform?"
fetch "$BASE/$SUM" "$TMP/$SUM"          || die "could not download the checksum for $TARBALL."
ok "downloaded" "$TARBALL"

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
ok "verified" "sha256 checksum"

tar -xzf "$TMP/$TARBALL" -C "$TMP"
[ -f "$TMP/reaper" ] || die "archive did not contain the reaper binary."
mkdir -p "$BIN_DIR"
install -m 755 "$TMP/reaper" "$BIN_DIR/reaper"
ok "installed" "$BIN_DIR/reaper"

line ""
case ":$PATH:" in
  *":$BIN_DIR:"*)
    line "  ${b}reaper${rst} is ready — run ${b}reaper${rst} to list listening ports."
    line "  ${dim}tip: 'sudo reaper' includes other users' processes.${rst}"
    ;;
  *)
    warn "${b}$BIN_DIR${rst} is not on your PATH yet."
    line "     this session:  ${b}export PATH=\"$BIN_DIR:\$PATH\"${rst}"
    line "     to keep it:     append that line to ~/.bashrc or ~/.zshrc"
    line "  then run ${b}reaper${rst} to list listening ports (${b}sudo reaper${rst} for all users)."
    ;;
esac
