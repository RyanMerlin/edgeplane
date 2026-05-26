#!/usr/bin/env bash
set -euo pipefail

EP_BASE_URL="${EP_BASE_URL:?EP_BASE_URL must be set — e.g. https://your-edgeplane.example.com}"
EP_TOKEN="${EP_TOKEN:?EP_TOKEN must be set — get your token from the EdgePlane admin}"
echo "Installing edgeplane CLI..."
bash "$(dirname "$0")/install-edgeplane.sh"
export PATH="$HOME/.local/bin:$PATH"
if ! command -v edgeplane >/dev/null 2>&1; then
  echo "edgeplane not found on PATH after install" >&2
  exit 1
fi

if command -v tailscale >/dev/null 2>&1; then
  if ! tailscale status >/dev/null 2>&1; then
    echo "warning: tailscale is installed but not healthy/running; agent may not reach ${EP_BASE_URL}" >&2
  fi
fi

ENV_FILE="$HOME/.edgeplane-agent.env"
cat > "$ENV_FILE" <<EOV
export EP_BASE_URL="$EP_BASE_URL"
export EP_TOKEN="$EP_TOKEN"
EOV

chmod 600 "$ENV_FILE"

echo "Done."
echo ""
echo "1) Load env vars:"
echo "   source $ENV_FILE"
echo ""
echo "2) MCP config snippet:"
cat <<EOC
{
  "edgeplane": {
    "command": "edgeplane",
    "args": ["serve"],
    "env": {
      "EP_BASE_URL": "$EP_BASE_URL",
      "EP_TOKEN": "$EP_TOKEN"
    }
  }
}
EOC
