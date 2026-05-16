#!/usr/bin/env bash
#
# Install mc (CLI) and mcd (per-node daemon) from the latest GitHub
# Release. Falls back to building from source if a matching binary
# isn't published for your platform. Optionally installs a systemd user
# unit so mcd starts on login.
#
# Quickstart (no flags):
#   bash scripts/install-mc.sh
#
# Common options:
#   --prefix DIR        Install binaries here (default: ~/.local/bin)
#   --install-service   Also install + enable the mcd systemd user unit
#   --no-daemon           Skip mcd; install just the mc CLI
#   --version TAG       Pin a specific release tag (default: latest)
#   --env-file FILE     Load env from FILE in the systemd unit
#
# Env overrides:
#   MC_INSTALL_PREFIX, MC_INSTALL_VERSION, MC_INSTALL_NO_DAEMON=1,
#   MC_INSTALL_SERVICE=1, MC_ENV_FILE, MC_INSTALL_SHELL_HOOK=1.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

PREFIX="${MC_INSTALL_PREFIX:-$HOME/.local/bin}"
VERSION="${MC_INSTALL_VERSION:-latest}"
INSTALL_DAEMON=1
INSTALL_SERVICE="${MC_INSTALL_SERVICE:-0}"
ENV_FILE="${MC_ENV_FILE:-$ROOT_DIR/.env}"
AUTO_SHELL_HOOK="${MC_INSTALL_SHELL_HOOK:-0}"

[[ "${MC_INSTALL_NO_DAEMON:-0}" = "1" ]] && INSTALL_DAEMON=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --prefix)          PREFIX="$2"; shift 2 ;;
    --version)         VERSION="$2"; shift 2 ;;
    --no-daemon)         INSTALL_DAEMON=0; shift ;;
    --install-service) INSTALL_SERVICE=1; shift ;;
    --env-file)        ENV_FILE="$2"; shift 2 ;;
    -h|--help)
      sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      exit 1
      ;;
  esac
done

mkdir -p "$PREFIX"

# ── Platform detection ────────────────────────────────────────────────────────

detect_platform() {
  local os arch
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"
  case "$arch" in
    x86_64)        arch="x86_64" ;;
    aarch64|arm64) arch="aarch64" ;;
    *) return 1 ;;
  esac
  case "$os" in
    linux|darwin) ;;
    *) return 1 ;;
  esac
  echo "${os}-${arch}"
}

PLATFORM=""
if PLATFORM=$(detect_platform); then
  echo "detected platform: ${PLATFORM}"
else
  echo "unrecognised platform — will build from source if possible" >&2
fi

# ── Download or build a binary ────────────────────────────────────────────────
#
# Args: bin_name crate_path target_path
# Tries the release first; on miss, builds from source via cargo.

install_binary() {
  local bin="$1" crate_path="$2" target_path="$3"
  local artifact="${bin}-${PLATFORM}"
  local base_url

  if [[ "$VERSION" = "latest" ]]; then
    base_url="https://github.com/RyanMerlin/missioncontrol/releases/latest/download"
  else
    base_url="https://github.com/RyanMerlin/missioncontrol/releases/download/${VERSION}"
  fi

  if [[ -n "$PLATFORM" ]] && curl -fsSL --max-time 30 -o "${target_path}.tmp" "${base_url}/${artifact}" 2>/dev/null; then
    # Verify checksum if checksums.txt is reachable.
    if curl -fsSL --max-time 10 -o "${target_path}.checksums" "${base_url}/checksums.txt" 2>/dev/null; then
      local expected actual
      expected="$(grep " ${artifact}$" "${target_path}.checksums" | awk '{print $1}' || true)"
      if [[ -n "$expected" ]]; then
        if command -v sha256sum >/dev/null 2>&1; then
          actual="$(sha256sum "${target_path}.tmp" | awk '{print $1}')"
        else
          actual="$(shasum -a 256 "${target_path}.tmp" | awk '{print $1}')"
        fi
        if [[ "$expected" != "$actual" ]]; then
          echo "checksum mismatch for $artifact — aborting" >&2
          rm -f "${target_path}.tmp" "${target_path}.checksums"
          exit 1
        fi
        echo "  verified checksum"
      fi
      rm -f "${target_path}.checksums"
    fi
    mv "${target_path}.tmp" "$target_path"
    chmod +x "$target_path"
    echo "  installed $bin from release ${VERSION}"
    return 0
  fi

  rm -f "${target_path}.tmp"
  echo "  release binary unavailable for $artifact — building from source"
  if ! command -v cargo >/dev/null 2>&1; then
    echo "  cargo is required to build $bin — install Rust from https://rustup.rs" >&2
    exit 1
  fi
  local extra_features=""
  if [[ "$bin" = "mc" ]]; then extra_features="--features tui"; fi
  (
    cd "${ROOT_DIR}/${crate_path}"
    cargo build --release $extra_features
  )
  cp "${ROOT_DIR}/${crate_path}/target/release/${bin}" "$target_path"
  chmod +x "$target_path"
  echo "  built and installed $bin from source"
}

# ── Install mc ────────────────────────────────────────────────────────────────

echo "installing mc to ${PREFIX}/mc"
install_binary mc integrations/mc "${PREFIX}/mc"
"${PREFIX}/mc" --version

# ── Install mcd ───────────────────────────────────────────────────────────

if [[ "$INSTALL_DAEMON" = "1" ]]; then
  echo "installing mcd to ${PREFIX}/mcd"
  install_binary mcd integrations/mcd "${PREFIX}/mcd"
  if "${PREFIX}/mcd" --version >/dev/null 2>&1; then
    echo "  $("${PREFIX}/mcd" --version)"
  fi
else
  echo "skipping mcd (per --no-daemon)"
fi

# ── Optional: systemd user unit for mcd ───────────────────────────────────

if [[ "$INSTALL_SERVICE" = "1" && "$INSTALL_DAEMON" = "1" ]]; then
  if ! command -v systemctl >/dev/null 2>&1; then
    echo "systemctl not found — skipping service install" >&2
  else
    UNIT_DIR="${HOME}/.config/systemd/user"
    UNIT_FILE="${UNIT_DIR}/mcd.service"
    SRC_UNIT="${ROOT_DIR}/integrations/mcd/systemd/mcd.service"
    mkdir -p "$UNIT_DIR" "${HOME}/.mc/mcd"

    # Remove legacy mc-mesh unit if present (clean cutover).
    LEGACY_UNIT="${UNIT_DIR}/mc-mesh.service"
    if [[ -f "$LEGACY_UNIT" ]]; then
      echo "removing legacy mc-mesh.service unit…"
      systemctl --user disable --now mc-mesh.service 2>/dev/null || true
      rm -f "$LEGACY_UNIT"
      systemctl --user daemon-reload || true
    fi

    if [[ -f "$SRC_UNIT" ]]; then
      # Rewrite the ExecStart to point at our actual install prefix.
      sed "s|%h/\.cargo/bin/mcd|${PREFIX}/mcd|g" "$SRC_UNIT" > "$UNIT_FILE"
      # If an env file was given, add a corresponding EnvironmentFile line
      # (idempotent — leaves the unit alone if already present).
      if [[ -f "$ENV_FILE" ]] && ! grep -q "EnvironmentFile=" "$UNIT_FILE"; then
        sed -i "/^\[Service\]/a EnvironmentFile=${ENV_FILE}" "$UNIT_FILE"
      fi
      echo "installed systemd user unit: $UNIT_FILE"

      systemctl --user daemon-reload || true
      if systemctl --user enable --now mcd.service 2>&1; then
        echo "enabled + started mcd.service"
        systemctl --user --no-pager status mcd.service | head -5 || true
      else
        echo "warning: could not enable mcd.service — check 'systemctl --user status mcd'" >&2
      fi
    else
      echo "unit template not found at $SRC_UNIT — skipping service install" >&2
    fi
  fi
fi

# ── Optional: shell env hook ──────────────────────────────────────────────────

append_shell_hook() {
  local rc_file="$1"
  local marker_begin="# >>> missioncontrol mc env >>>"
  local marker_end="# <<< missioncontrol mc env <<<"
  [[ ! -f "$rc_file" ]] && touch "$rc_file"
  if grep -Fq "$marker_begin" "$rc_file"; then
    return 0
  fi
  cat >>"$rc_file" <<EOF
$marker_begin
if [ -f "$ENV_FILE" ]; then
  set -a
  . "$ENV_FILE"
  set +a
fi
$marker_end
EOF
  echo "installed shell hook in $rc_file"
}

if [[ "$AUTO_SHELL_HOOK" = "1" && -f "$ENV_FILE" ]]; then
  append_shell_hook "$HOME/.zshrc"
  append_shell_hook "$HOME/.bashrc"
fi

# ── Done ──────────────────────────────────────────────────────────────────────

cat <<DONE

Installed:
  mc       → ${PREFIX}/mc
$( [[ "$INSTALL_DAEMON" = "1" ]] && echo "  mcd  → ${PREFIX}/mcd" )

Next steps:
  1. Sign in:                mc auth login
  2. Launch the TUI:         mc tui
$( [[ "$INSTALL_SERVICE" = "1" && "$INSTALL_DAEMON" = "1" ]] && cat <<SERVICE
  3. mcd is running as a systemd user service.
     Check status:           systemctl --user status mcd
     Tail logs:              journalctl --user -u mcd -f
SERVICE
)$( [[ "$INSTALL_SERVICE" != "1" && "$INSTALL_DAEMON" = "1" ]] && cat <<MANUAL
  3. Start mcd manually (or re-run with --install-service):
     mcd run
MANUAL
)

Make sure ${PREFIX} is on your PATH.
DONE
