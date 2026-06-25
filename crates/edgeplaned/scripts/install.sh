#!/usr/bin/env bash
# install.sh — bootstrap edgeplaned on a fresh machine
#
# Usage:
#   bash crates/edgeplaned/scripts/install.sh
#
# What it does:
#   1. Checks for Rust / cargo
#   2. Builds edgeplaned from source
#   3. Installs the edgeplane binary (if not already installed)
#   4. Optionally installs the systemd user unit

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
EP_DIR="$REPO_ROOT/crates/edgeplaned"
EP_DIR="$REPO_ROOT/crates/edgeplane"

green()  { printf '\033[0;32m%s\033[0m\n' "$*"; }
yellow() { printf '\033[0;33m%s\033[0m\n' "$*"; }
red()    { printf '\033[0;31m%s\033[0m\n' "$*"; }
die()    { red "Error: $*"; exit 1; }

# ---------------------------------------------------------------------------
# 1. Prerequisites
# ---------------------------------------------------------------------------
command -v cargo >/dev/null 2>&1 || die "cargo not found. Install Rust: https://rustup.rs"

green "→ Building edgeplaned…"
cargo install --path "$EP_DIR/crates/edgeplaned" --quiet

green "→ Building edgeplane (CLI)…"
cargo install --path "$EP_DIR" --quiet

# ---------------------------------------------------------------------------
# 2. Verify installation
# ---------------------------------------------------------------------------
EP_BIN="$(command -v edgeplaned 2>/dev/null || true)"
EP_BIN="$(command -v edgeplane 2>/dev/null || true)"

[[ -n "$EP_BIN" ]] || die "edgeplaned binary not found after install — check \$PATH"
[[ -n "$EP_BIN" ]]      || die "edgeplane binary not found after install — check \$PATH"

green "✓ edgeplaned installed at $EP_BIN"
green "✓ edgeplane installed at $EP_BIN"

# ---------------------------------------------------------------------------
# 3. Create config / work dirs
# ---------------------------------------------------------------------------
mkdir -p "$HOME/.edgeplane/edgeplaned/work"
mkdir -p "$HOME/.config/systemd/user"

# ---------------------------------------------------------------------------
# 4. Systemd user unit (optional)
# ---------------------------------------------------------------------------
if command -v systemctl >/dev/null 2>&1; then
    UNIT_PATH="$HOME/.config/systemd/user/edgeplaned.service"
    if [[ ! -f "$UNIT_PATH" ]]; then
        read -r -p "Install systemd user unit so edgeplaned starts on login? [y/N] " ans
        if [[ "${ans,,}" == "y" || "${ans,,}" == "yes" ]]; then
            cat > "$UNIT_PATH" <<EOF
[Unit]
Description=edgeplaned agent coordination daemon
After=network.target

[Service]
ExecStart=$EP_BIN run
Restart=on-failure
RestartSec=5s
StandardOutput=journal
StandardError=journal
SyslogIdentifier=edgeplaned

[Install]
WantedBy=default.target
EOF
            systemctl --user daemon-reload
            systemctl --user enable edgeplaned.service
            green "✓ Systemd user unit installed and enabled."
        fi
    else
        yellow "! Systemd unit already exists at $UNIT_PATH, skipping."
    fi
fi

# ---------------------------------------------------------------------------
# 5. Print next steps
# ---------------------------------------------------------------------------
echo ""
green "Installation complete!"
echo ""
echo "Next steps:"
echo "  1. Add credentials to ~/.edgeplane/edgeplaned.yaml"
echo "     (see crates/edgeplaned/README.md for the schema and usage)"
echo ""
echo "  2. Start the daemon:"
echo "     edgeplane daemon up"
echo ""
echo "  3. Install agent runtimes:"
echo "     edgeplane daemon runtime install claude-code"
echo "     edgeplane daemon runtime install codex"
echo "     edgeplane daemon runtime install gemini"
echo ""
echo "  4. Enroll agents and run tasks:"
echo "     edgeplane daemon agent enroll --domain <id> --runtime claude-code"
echo "     edgeplane daemon task run <mission-id> --title 'my first task'"
echo ""
echo "  5. Watch progress:"
echo "     edgeplane daemon watch --domain <id>"
