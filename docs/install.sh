#!/bin/sh
# reaper installer — https://reaper.aymenkrifa.com
#   curl -LsSf https://reaper.aymenkrifa.com/install.sh | sh
#
# Downloads the static musl binary for this machine from the latest GitHub
# release, verifies it against the published sha256, and drops it in ~/.local/bin.
# Re-run any time to update — it tells you which version you came from and
# landed on. Override the destination with REAPER_BIN_DIR=/somewhere, or
# silence the progress lines with REAPER_QUIET=1 (errors still print).
set -eu

# Everything below is function definitions only; nothing executes until the
# `main "$@"` on the last line. If the download is cut off mid-transfer, a
# truncated script is a syntax error or a no-op — it can never run half an
# install.

quiet() { [ -n "${REAPER_QUIET:-}" ]; }
line() { quiet || printf '%s\n' "$*"; }
ok()   { quiet || printf '  %s✓%s %-10s %s\n' "$grn" "$rst" "$1" "$2"; }
warn() { quiet || printf '  %s!%s %s\n' "$ylw" "$rst" "$*"; }
die()  { printf '\n  %serror%s %s\n' "${red:-}" "${rst:-}" "$*" >&2; exit 1; }

fetch() { curl --proto '=https' --tlsv1.2 -fsSL "$1" -o "$2"; }

# sha256 of a file, with whichever tool this system has.
hash_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

# Print the version a reaper binary reports, or nothing. Binaries up to
# v0.3.1 had no --version and start the TUI instead: setsid detaches them
# from the terminal so their raw-mode setup fails instantly rather than
# hijacking the screen mid-install. No setsid → skip probing entirely.
probe_version() {
  [ -x "$1" ] || return 0
  command -v setsid >/dev/null 2>&1 || return 0
  setsid "$1" --version </dev/null 2>/dev/null | awk 'NR==1 && $1=="reaper" {print $2}' || true
}

# Closing guidance — shared by the early "up to date" exit and the full
# install path. Warns when this reaper isn't the one PATH resolves to.
closing() {
  line ""
  case ":$PATH:" in
    *":$BIN_DIR:"*)
      # A different reaper earlier on PATH would silently keep winning.
      shadow="$(command -v reaper 2>/dev/null || true)"
      if [ -n "$shadow" ] && [ "$shadow" != "$BIN_DIR/reaper" ]; then
        warn "another reaper at ${b}$shadow${rst} comes first on your PATH and will shadow this one."
      fi
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
}

main() {
  REPO="aymenkrifa/reaper"
  BASE="https://github.com/$REPO/releases/latest/download"

  # Readable output: colour only on a real terminal (honours NO_COLOR), plain
  # when piped to a file. REAPER_QUIET=1 hushes everything but errors, which
  # always go to stderr.
  if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    b=$(printf '\033[1m');   dim=$(printf '\033[2m'); grn=$(printf '\033[32m')
    ylw=$(printf '\033[33m'); red=$(printf '\033[31m'); rst=$(printf '\033[0m')
  else
    b=; dim=; grn=; ylw=; red=; rst=
  fi

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

  [ "$(uname -s)" = "Linux" ] || die "reaper is Linux-only (it reads /proc)."
  case "$(uname -m)" in
    x86_64 | amd64)  ARCH=x86_64 ;;
    aarch64 | arm64) ARCH=aarch64 ;;
    *) die "unsupported architecture: $(uname -m)" ;;
  esac

  command -v curl >/dev/null 2>&1 || die "curl is required."
  command -v tar  >/dev/null 2>&1 || die "tar is required."
  command -v sha256sum >/dev/null 2>&1 || command -v shasum >/dev/null 2>&1 \
    || die "need sha256sum or shasum to verify the download."

  TARBALL="reaper-$ARCH-unknown-linux-musl.tar.gz"
  SUM="reaper-$ARCH-unknown-linux-musl.sha256" # taiki-e drops .tar.gz before .sha256
  TMP="$(mktemp -d)"
  trap 'rm -rf "$TMP"' EXIT

  line ""
  line "  ${b}reaper${rst}${dim} · a linux tui for listing & killing listening ports${rst}"
  line ""
  ok "target" "$ARCH-unknown-linux-musl"

  # What's already installed, so the end of the run can say whether this
  # was a fresh install, an update, or a no-op.
  old_sum=""; old_ver=""
  if [ -f "$BIN_DIR/reaper" ]; then
    old_sum="$(hash_file "$BIN_DIR/reaper")"
    old_ver="$(probe_version "$BIN_DIR/reaper")"
  fi

  # When the installed binary can say what version it is, one redirect
  # (/releases/latest → /releases/tag/vX, no download) tells us the latest
  # release tag — matching versions stop here instead of downloading a
  # tarball just to hash-compare it. Binaries older than 0.3.2 can't
  # report a version, and any hiccup in the redirect leaves latest_url
  # unmatched: both fall through to the full download-and-verify path.
  if [ -n "$old_ver" ]; then
    latest_url="$(curl --proto '=https' --tlsv1.2 -fsSLI -o /dev/null -w '%{url_effective}' \
      "https://github.com/$REPO/releases/latest" 2>/dev/null || true)"
    case "$latest_url" in
      */releases/tag/*)
        latest="${latest_url##*/tag/}"
        latest="${latest#v}"
        if [ "$latest" = "$old_ver" ]; then
          ok "up to date" "already on the latest release (v$old_ver)"
          closing
          return 0
        fi
        ;;
    esac
  fi

  fetch "$BASE/$TARBALL" "$TMP/$TARBALL" || die "could not download $TARBALL — is there a release for your platform?"
  fetch "$BASE/$SUM" "$TMP/$SUM"          || die "could not download the checksum for $TARBALL."
  ok "downloaded" "$TARBALL"

  expected="$(awk 'NR==1{print $1}' "$TMP/$SUM")"
  [ -n "$expected" ] || die "could not read the published checksum."
  actual="$(hash_file "$TMP/$TARBALL")"
  [ "$expected" = "$actual" ] || die "checksum mismatch — refusing to install."
  ok "verified" "sha256 checksum"

  tar -xzf "$TMP/$TARBALL" -C "$TMP"
  [ -f "$TMP/reaper" ] || die "archive did not contain the reaper binary."
  new_ver="$(probe_version "$TMP/reaper")"
  new_sum="$(hash_file "$TMP/reaper")"
  mkdir -p "$BIN_DIR"
  install -m 755 "$TMP/reaper" "$BIN_DIR/reaper"

  if [ -z "$old_sum" ]; then
    ok "installed" "$BIN_DIR/reaper${new_ver:+  (v$new_ver)}"
  elif [ "$old_sum" = "$new_sum" ]; then
    ok "up to date" "already on the latest release${new_ver:+ (v$new_ver)}"
  elif [ -n "$new_ver" ]; then
    ok "updated" "${old_ver:+v$old_ver }→ v$new_ver"
  else
    ok "updated" "$BIN_DIR/reaper"
  fi

  closing
}

main "$@"
