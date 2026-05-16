#!/usr/bin/env bash
# install.sh — bootstrap mcd on a fresh machine
#
# Usage:
#   bash integrations/mcd/scripts/install.sh
#
# What it does:
#   1. Checks for Rust / cargo
#   2. Builds mcd from source
#   3. Installs the mc binary (if not already installed)
#   4. Optionally installs the systemd user unit

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
MCD_DIR="$REPO_ROOT/integrations/mcd"
MC_DIR="$REPO_ROOT/integrations/mc"

green()  { printf '\033[0;32m%s\033[0m\n' "$*"; }
yellow() { printf '\033[0;33m%s\033[0m\n' "$*"; }
red()    { printf '\033[0;31m%s\033[0m\n' "$*"; }
die()    { red "Error: $*"; exit 1; }

# ---------------------------------------------------------------------------
# 1. Prerequisites
# ---------------------------------------------------------------------------
command -v cargo >/dev/null 2>&1 || die "cargo not found. Install Rust: https://rustup.rs"

green "→ Building mcd…"
cargo install --path "$MCD_DIR/crates/mcd" --quiet

green "→ Building mc (CLI)…"
cargo install --path "$MC_DIR" --quiet

# ---------------------------------------------------------------------------
# 2. Verify installation
# ---------------------------------------------------------------------------
MCD_BIN="$(command -v mcd 2>/dev/null || true)"
MC_BIN="$(command -v mc 2>/dev/null || true)"

[[ -n "$MCD_BIN" ]] || die "mcd binary not found after install — check \$PATH"
[[ -n "$MC_BIN" ]]      || die "mc binary not found after install — check \$PATH"

green "✓ mcd installed at $MCD_BIN"
green "✓ mc installed at $MC_BIN"

# ---------------------------------------------------------------------------
# 3. Create config / work dirs
# ---------------------------------------------------------------------------
mkdir -p "$HOME/.missioncontrol/mcd/work"
mkdir -p "$HOME/.config/systemd/user"

# ---------------------------------------------------------------------------
# 4. Systemd user unit (optional)
# ---------------------------------------------------------------------------
if command -v systemctl >/dev/null 2>&1; then
    UNIT_PATH="$HOME/.config/systemd/user/mcd.service"
    if [[ ! -f "$UNIT_PATH" ]]; then
        read -r -p "Install systemd user unit so mcd starts on login? [y/N] " ans
        if [[ "${ans,,}" == "y" || "${ans,,}" == "yes" ]]; then
            cat > "$UNIT_PATH" <<EOF
[Unit]
Description=mcd agent coordination daemon
After=network.target

[Service]
ExecStart=$MCD_BIN run
Restart=on-failure
RestartSec=5s
StandardOutput=journal
StandardError=journal
SyslogIdentifier=mcd

[Install]
WantedBy=default.target
EOF
            systemctl --user daemon-reload
            systemctl --user enable mcd.service
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
echo "  1. Add credentials to ~/.missioncontrol/mcd.yaml"
echo "     (see integrations/mcd/README.md for the schema and usage)"
echo ""
echo "  2. Start the daemon:"
echo "     mc mesh up"
echo ""
echo "  3. Install agent runtimes:"
echo "     mc mesh runtime install claude-code"
echo "     mc mesh runtime install codex"
echo "     mc mesh runtime install gemini"
echo ""
echo "  4. Enroll agents and run tasks:"
echo "     mc mesh agent enroll --mission <id> --runtime claude-code"
echo "     mc mesh task run <kluster-id> --title 'my first task'"
echo ""
echo "  5. Watch progress:"
echo "     mc mesh watch --mission <id>"
