#!/usr/bin/env bash
set -euo pipefail

PREFIX="${MC_INSTALL_PREFIX:-$HOME/.local/bin}"
TARGET="${MC_INSTALL_TARGET:-$PREFIX/mc}"
ENV_FILE="${MC_ENV_FILE:-$HOME/.missioncontrol-agent.env}"
AUTO_SHELL_HOOK="${MC_INSTALL_SHELL_HOOK:-1}"
BASE_URL="${MC_RELEASE_BASE_URL:-https://github.com/RyanMerlin/missioncontrol/releases/latest/download}"

append_shell_hook() {
  local rc_file="$1"
  local marker_begin="# >>> missioncontrol mc env >>>"
  local marker_end="# <<< missioncontrol mc env <<<"
  if [[ ! -f "$rc_file" ]]; then
    touch "$rc_file"
  fi
  if grep -Fq "$marker_begin" "$rc_file"; then
    echo "shell hook already present in $rc_file"
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

try_download_release() {
  local os arch artifact
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"
  case "$arch" in
    x86_64) arch="x86_64" ;;
    aarch64|arm64) arch="aarch64" ;;
    *) return 1 ;;
  esac
  case "$os" in
    linux|darwin) ;;
    *) return 1 ;;
  esac

  case "$os" in
    linux) artifact="mc-linux-${arch}" ;;
    darwin) artifact="mc-macos-${arch}" ;;
  esac

  echo "trying binary download: ${BASE_URL}/${artifact}"
  if ! curl -fsSL --max-time 30 -o "$TARGET.tmp" "${BASE_URL}/${artifact}" 2>/dev/null; then
    rm -f "$TARGET.tmp"
    return 1
  fi

  if curl -fsSL --max-time 10 -o "$TARGET.checksums" "${BASE_URL}/checksums.txt" 2>/dev/null; then
    local expected actual
    expected="$(grep " ${artifact}$" "$TARGET.checksums" | awk '{print $1}')"
    if [[ -n "$expected" ]]; then
      if command -v sha256sum >/dev/null 2>&1; then
        actual="$(sha256sum "$TARGET.tmp" | awk '{print $1}')"
      else
        actual="$(shasum -a 256 "$TARGET.tmp" | awk '{print $1}')"
      fi
      if [[ "$expected" != "$actual" ]]; then
        echo "checksum mismatch — aborting install" >&2
        rm -f "$TARGET.tmp" "$TARGET.checksums"
        return 1
      fi
      echo "checksum verified"
    fi
    rm -f "$TARGET.checksums"
  fi

  mv "$TARGET.tmp" "$TARGET"
  chmod +x "$TARGET"
  echo "installed mc from release binary"
  return 0
}

mkdir -p "$PREFIX"

if ! try_download_release; then
  echo "binary download unavailable, building from source (requires cargo)..."
  if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo is required to build mc — install Rust from https://rustup.rs" >&2
    exit 1
  fi

  # When running via `bash <(curl ...)`, BASH_SOURCE[0] is a /proc/self/fd path, not a real file.
  # Detect that case and clone the repo to a tmpdir instead.
  SCRIPT_PATH="${BASH_SOURCE[0]:-}"
  if [[ -f "$SCRIPT_PATH" ]]; then
    ROOT_DIR="$(cd "$(dirname "$SCRIPT_PATH")/.." && pwd)"
    CLEANUP_ROOT=""
  else
    if ! command -v git >/dev/null 2>&1; then
      echo "git is required to clone the source — install git and retry" >&2
      exit 1
    fi
    ROOT_DIR="$(mktemp -d)"
    CLEANUP_ROOT="$ROOT_DIR"
    echo "cloning missioncontrol to build from source..."
    git clone --depth 1 https://github.com/RyanMerlin/missioncontrol.git "$ROOT_DIR"
  fi

  (
    cd "$ROOT_DIR/integrations/mc"
    cargo build --release
  )
  cp "$ROOT_DIR/integrations/mc/target/release/mc" "$TARGET"
  chmod +x "$TARGET"

  if [[ -n "$CLEANUP_ROOT" ]]; then
    rm -rf "$CLEANUP_ROOT"
  fi
fi

mkdir -p "$(dirname "$ENV_FILE")"
if [[ ! -f "$ENV_FILE" ]]; then
  cat >"$ENV_FILE" <<EOF
# MissionControl shell environment
export MC_INSTALL_PREFIX="$PREFIX"
export MC_BASE_URL="${MC_BASE_URL:-https://missioncontrol.example.com}"
export MC_TOKEN="${MC_TOKEN:-}"
EOF
  chmod 0600 "$ENV_FILE"
fi

echo "installed mc to $TARGET"
if command -v mc >/dev/null 2>&1; then
  echo "mc on PATH: $(command -v mc)"
fi
"$TARGET" --version

if [[ "$AUTO_SHELL_HOOK" == "1" ]]; then
  append_shell_hook "$HOME/.zshrc"
  append_shell_hook "$HOME/.bashrc"
  echo "auto env loading enabled from $ENV_FILE"
else
  echo "Optional: enable auto env loading into new shells"
  echo "  MC_INSTALL_SHELL_HOOK=1 MC_ENV_FILE=$ENV_FILE bash <(curl -fsSL https://raw.githubusercontent.com/RyanMerlin/missioncontrol/main/scripts/bootstrap-mc.sh)"
fi

echo ""
echo "Launch an agent:"
echo "  source \"$ENV_FILE\""
echo "  mc claude run default"
echo "  mc codex run default"
