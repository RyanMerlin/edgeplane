#!/usr/bin/env bash
set -euo pipefail

EP_BASE_URL="${EP_BASE_URL:?EP_BASE_URL must be set — e.g. https://your-edgeplane.example.com}"
NODE_NAME="${NODE_NAME:?NODE_NAME must be set — a unique name for this node}"

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

echo "Registering node '${NODE_NAME}' with ${EP_BASE_URL}..."
edgeplane --base-url "$EP_BASE_URL" agent node register --node-name "$NODE_NAME"

echo ""
echo "Done. MCP config snippet:"
cat <<EOC
{
  "edgeplane": {
    "command": "edgeplane",
    "args": ["serve"],
    "env": {
      "EP_BASE_URL": "$EP_BASE_URL"
    }
  }
}
EOC
