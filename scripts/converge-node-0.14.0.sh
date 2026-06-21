#!/usr/bin/env bash
#
# converge-node-0.14.0.sh — one-shot remediation for edgeplane version drift.
#
# WHY THIS EXISTS
#   This node accumulated a v1/v2 split: the edgeplaned daemon + the per-profile
#   ACP agents run 0.13.x (v1 socket layout under ~/.edgeplane/edgeplaned/), while
#   a 0.14.0 CLI on PATH self-healed paths to the v2 layout (sockets under run/).
#   migrate.rs creates NO v1 socket compat shim, so a 0.14.0 daemon serves sockets
#   ONLY at run/. Therefore daemon and agents MUST be restarted together on the
#   same version — a daemon-only bump would strand the 0.13.x agents.
#
# RUN FROM A PLAIN SSH/TTY — *not* inside a managed agent zellij session.
#   This restarts every profile agent service (each hosts a live Claude session)
#   plus edgeplaned. If you run it from inside one of those sessions, it kills the
#   shell mid-run. The cgroup guard below refuses that case.
#
# Converges only NODE-LOCAL dirs (~/.cargo/bin, ~/.local/bin). It never writes the
# shared /workspace CephFS copy (other nodes share it).
set -euo pipefail

REPO="${EP_UPDATE_REPO:-edgeplane/edgeplane}"
BASE="https://github.com/${REPO}/releases/latest/download"
BIN_DIRS=("$HOME/.cargo/bin" "$HOME/.local/bin")
DAEMON_UNIT="edgeplaned.service"
# Override EP_PROFILE_UNITS with your fleet's agent service names.
# Example: PROFILE_UNITS=(my-agent-alpha.service my-agent-beta.service)
IFS=' ' read -r -a PROFILE_UNITS <<< "${EP_PROFILE_UNITS:-}"

log() { printf '[converge] %s\n' "$*" >&2; }
sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi; }

# ── Guard: refuse to run from inside a managed agent/daemon cgroup ─────────────
selfcg=$(cat /proc/self/cgroup 2>/dev/null || true)
case "$selfcg" in
  *edgeplaned.service*)
    log "REFUSING: you are inside a managed session ($selfcg)."
    log "Run this from a plain SSH/tty — it restarts the very session you're in."
    exit 2 ;;
esac

os=$(uname -s); arch=$(uname -m)
case "$os" in Linux) os=linux ;; Darwin) os=macos ;; *) log "unsupported OS: $os"; exit 1 ;; esac
case "$arch" in x86_64) arch=x86_64 ;; aarch64|arm64) arch=aarch64 ;; *) log "unsupported arch: $arch"; exit 1 ;; esac

tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT

# ── Download + verify the 0.14.x release binaries once ────────────────────────
log "fetching checksums + binaries for ${os}/${arch}"
curl -fsSL --retry 3 --max-time 60 -o "$tmp/checksums.txt" "${BASE}/checksums.txt"
fetch() {
  local name="$1" want got
  want=$(awk -v n="$name" '$2==n{print $1}' "$tmp/checksums.txt")
  [ -n "$want" ] || { log "no checksum entry for $name"; exit 1; }
  curl -fL --retry 3 --max-time 300 -o "$tmp/$name" "${BASE}/${name}"
  got=$(sha256 "$tmp/$name")
  [ "$want" = "$got" ] || { log "CHECKSUM MISMATCH $name (want $want got $got)"; exit 1; }
  chmod +x "$tmp/$name"
}
fetch "edgeplane-${os}-${arch}"
fetch "edgeplaned-${os}-${arch}"
newver=$("$tmp/edgeplane-${os}-${arch}" --version | awk '{print $2}')
log "release version: $newver"

# ── Stop the whole stack (agents first, then daemon) ──────────────────────────
log "stopping profile agents…"
for u in "${PROFILE_UNITS[@]}"; do systemctl --user stop "$u" 2>/dev/null || true; done
log "stopping $DAEMON_UNIT…"
systemctl --user stop "$DAEMON_UNIT" 2>/dev/null || true

# ── Swap binaries into every node-local dir that already has them ─────────────
for dir in "${BIN_DIRS[@]}"; do
  [ -d "$dir" ] || continue
  for bin in edgeplane edgeplaned; do
    [ -e "$dir/$bin" ] || continue
    install -m 0755 "$tmp/${bin}-${os}-${arch}" "$dir/$bin.new"
    mv -f "$dir/$bin.new" "$dir/$bin"
    log "  $dir/$bin → $newver"
  done
done

# ── Clear stale v2 sockets so the daemon binds clean (it recreates them) ──────
rm -f "$HOME/.edgeplane/run"/*.sock 2>/dev/null || true

# ── Start daemon, wait for it to come up, then the agents ─────────────────────
log "starting $DAEMON_UNIT…"
systemctl --user start "$DAEMON_UNIT"
for i in $(seq 1 15); do
  systemctl --user is-active --quiet "$DAEMON_UNIT" && [ -S "$HOME/.edgeplane/run/mgmt.sock" ] && break
  sleep 1
done
if ! systemctl --user is-active --quiet "$DAEMON_UNIT"; then
  log "ERROR: $DAEMON_UNIT did not come up — check journalctl --user -u $DAEMON_UNIT"; exit 1
fi
log "restarting profile agents…"
for u in "${PROFILE_UNITS[@]}"; do systemctl --user start "$u" 2>/dev/null || log "  (skip $u — not present)"; done

# ── Verify ────────────────────────────────────────────────────────────────────
log "verifying convergence…"
edgeplane --version || true
ls -1 "$HOME/.edgeplane/run/"*.sock 2>/dev/null | sed 's/^/  serving /' >&2 || true
log "done — daemon + agents converged to $newver"
