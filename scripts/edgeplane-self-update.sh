#!/usr/bin/env bash
#
# edgeplane-self-update — converge this node's edgeplane binaries to the latest
# published GitHub release, then restart the daemon only if it actually changed.
#
# Tracks the latest `v*` release tag (release cadence), NOT main. Updates the
# `edgeplane` CLI and the `edgeplaned` daemon in EP_UPDATE_BIN_DIR (default
# ~/.cargo/bin — the directory the edgeplaned user service runs from), verifies
# each artifact's sha256 against the release checksums.txt, swaps atomically,
# and restarts edgeplaned.service iff its bytes changed.
#
# Intentionally NODE-LOCAL: it only writes under $HOME. It never touches shared
# storage like /workspace, where per-node timers would clobber each other.
#
# BRIDGE: this script duplicates download/verify/swap logic that, as of edgeplane
# 0.14.1, lives in `edgeplane update` itself (which converges every installed
# edgeplane binary, not just the CLI). Once a release >=0.14.1 is deployed across
# the fleet, this script collapses to:  edgeplane update  +  a restart-on-change
# check on $SERVICE. Until then it bridges nodes still running the CLI-only updater.
#
# Env overrides:
#   EP_UPDATE_REPO        owner/repo            (default edgeplane/edgeplane)
#   EP_UPDATE_BIN_DIR     install dir           (default $HOME/.cargo/bin)
#   EP_UPDATE_SERVICE     daemon unit to bounce (default edgeplaned.service)
#   EP_UPDATE_NO_RESTART  set to 1 to stage new binaries without restarting the
#                         daemon (for testing / maintenance windows)
set -euo pipefail

REPO="${EP_UPDATE_REPO:-edgeplane/edgeplane}"
BIN_DIR="${EP_UPDATE_BIN_DIR:-$HOME/.cargo/bin}"
SERVICE="${EP_UPDATE_SERVICE:-edgeplaned.service}"
NO_RESTART="${EP_UPDATE_NO_RESTART:-0}"
BASE="https://github.com/${REPO}/releases/latest/download"

log() { printf '[edgeplane-self-update] %s\n' "$*" >&2; }

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'
  else shasum -a 256 "$1" | awk '{print $1}'; fi
}

# os/arch tokens must match the release artifact naming (Rust env::consts).
os=$(uname -s); arch=$(uname -m)
case "$os" in Linux) os=linux ;; Darwin) os=macos ;; *) log "unsupported OS: $os"; exit 1 ;; esac
case "$arch" in x86_64) arch=x86_64 ;; aarch64|arm64) arch=aarch64 ;; *) log "unsupported arch: $arch"; exit 1 ;; esac

tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT

log "checking ${REPO} latest release for ${os}/${arch}"
# Release infra being briefly unreachable is a soft skip, not a failure.
if ! curl -fsSL --retry 3 --max-time 60 -o "$tmp/checksums.txt" "${BASE}/checksums.txt"; then
  log "release/checksums.txt unreachable — skipping this run"; exit 0
fi

# Download $1 into $tmp and verify its sha256 against checksums.txt. Hard-fails
# (non-zero exit) on a missing entry, download error, or checksum mismatch so a
# poisoned or truncated artifact surfaces as a failed unit rather than a swap.
fetch() {
  local name="$1" out="$tmp/$1" want got
  want=$(awk -v n="$name" '$2==n{print $1}' "$tmp/checksums.txt")
  [ -n "$want" ] || { log "no checksum entry for $name — refusing to update"; exit 1; }
  curl -fL --retry 3 --max-time 300 -o "$out" "${BASE}/${name}" \
    || { log "download failed: $name"; exit 1; }
  got=$(sha256 "$out")
  [ "$want" = "$got" ] || { log "CHECKSUM MISMATCH for $name (want $want, got $got) — aborting"; exit 1; }
}

# Atomically replace BIN_DIR/<dest> with the freshly downloaded src if they
# differ. Returns 0 if it replaced the file, 1 if already identical.
swap() {
  local src="$1" dest="$2"
  if [ -f "$dest" ] && cmp -s "$src" "$dest"; then return 1; fi
  install -m 0755 "$src" "$dest.new"
  mv -f "$dest.new" "$dest"   # atomic rename within BIN_DIR; running daemon keeps its old inode
  return 0
}

mkdir -p "$BIN_DIR"
fetch "edgeplane-${os}-${arch}"
fetch "edgeplaned-${os}-${arch}"

if swap "$tmp/edgeplane-${os}-${arch}" "$BIN_DIR/edgeplane"; then
  log "edgeplane CLI updated → $("$BIN_DIR/edgeplane" --version 2>/dev/null || echo '?')"
else
  log "edgeplane CLI already current"
fi

daemon_changed=0
if swap "$tmp/edgeplaned-${os}-${arch}" "$BIN_DIR/edgeplaned"; then daemon_changed=1; fi

if [ "$daemon_changed" -eq 1 ] && [ "$NO_RESTART" = "1" ]; then
  log "edgeplaned binary staged but EP_UPDATE_NO_RESTART=1 — not restarting $SERVICE"
elif [ "$daemon_changed" -eq 1 ]; then
  if systemctl --user is-active --quiet "$SERVICE"; then
    log "edgeplaned binary changed — restarting $SERVICE"
    systemctl --user restart "$SERVICE"
    sleep 2
    if systemctl --user is-active --quiet "$SERVICE"; then
      log "restart OK → $("$BIN_DIR/edgeplaned" --version 2>/dev/null || echo '?')"
    else
      log "WARNING: $SERVICE did not come back after restart — check journalctl --user -u $SERVICE"
      exit 1
    fi
  else
    log "edgeplaned binary updated; $SERVICE not active — leaving stopped"
  fi
else
  log "edgeplaned already current"
fi
log "done"
