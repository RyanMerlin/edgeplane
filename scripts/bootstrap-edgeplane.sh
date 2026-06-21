#!/usr/bin/env bash
set -euo pipefail

PREFIX="${EP_INSTALL_PREFIX:-$HOME/.local/bin}"
TARGET="${EP_INSTALL_TARGET:-$PREFIX/edgeplane}"
ENV_FILE="${EP_ENV_FILE:-$HOME/.edgeplane-agent.env}"
AUTO_SHELL_HOOK="${EP_INSTALL_SHELL_HOOK:-1}"
BASE_URL="${EP_RELEASE_BASE_URL:-https://github.com/edgeplane/edgeplane/releases/latest/download}"

append_shell_hook() {
  local rc_file="$1"
  local marker_begin="# >>> edgeplane edgeplane env >>>"
  local marker_end="# <<< edgeplane edgeplane env <<<"
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
    linux) artifact="edgeplane-linux-${arch}" ;;
    darwin) artifact="edgeplane-macos-${arch}" ;;
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
  echo "installed edgeplane from release binary"
  return 0
}

mkdir -p "$PREFIX"

if ! try_download_release; then
  echo "binary download unavailable, building from source..."
  if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo not found — installing Rust via rustup..."
    if ! curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path; then
      echo "rustup install failed — install Rust manually from https://rustup.rs" >&2
      exit 1
    fi
    # Activate the just-installed toolchain in this shell
    # shellcheck disable=SC1091
    source "${CARGO_HOME:-$HOME/.cargo}/env"
    if ! command -v cargo >/dev/null 2>&1; then
      echo "cargo still not found after rustup install — open a new shell and retry" >&2
      exit 1
    fi
    echo "Rust installed successfully"
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
    echo "cloning edgeplane to build from source..."
    git clone --depth 1 https://github.com/edgeplane/edgeplane.git "$ROOT_DIR"
  fi

  (
    cd "$ROOT_DIR/crates/edgeplane"
    cargo build --release
  )
  cp "$ROOT_DIR/crates/edgeplane/target/release/edgeplane" "$TARGET"
  chmod +x "$TARGET"

  if [[ -n "$CLEANUP_ROOT" ]]; then
    rm -rf "$CLEANUP_ROOT"
  fi
fi

mkdir -p "$(dirname "$ENV_FILE")"
if [[ ! -f "$ENV_FILE" ]]; then
  cat >"$ENV_FILE" <<EOF
# Edgeplane shell environment
export EP_INSTALL_PREFIX="$PREFIX"
export EP_BASE_URL="${EP_BASE_URL:-https://edgeplane.example.com}"
export EP_AGENT_TOKEN="${EP_AGENT_TOKEN:-}"
EOF
  chmod 0600 "$ENV_FILE"
fi

echo "installed edgeplane to $TARGET"
if command -v edgeplane >/dev/null 2>&1; then
  echo "edgeplane on PATH: $(command -v edgeplane)"
fi
"$TARGET" --version

if [[ "$AUTO_SHELL_HOOK" == "1" ]]; then
  append_shell_hook "$HOME/.zshrc"
  append_shell_hook "$HOME/.bashrc"
  echo "auto env loading enabled from $ENV_FILE"
else
  echo "Optional: enable auto env loading into new shells"
  echo "  EP_INSTALL_SHELL_HOOK=1 EP_ENV_FILE=$ENV_FILE bash <(curl -fsSL https://raw.githubusercontent.com/edgeplane/edgeplane/main/scripts/bootstrap-edgeplane.sh)"
fi

echo ""
echo "Launch an agent:"
echo "  source \"$ENV_FILE\""
echo "  edgeplane claude run default"
echo "  edgeplane codex run default"
