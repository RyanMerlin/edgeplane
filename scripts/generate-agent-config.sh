#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${EP_BASE_URL:-http://localhost:8008}"
AGENT="all"
OUT_DIR="./generated-agent-config"

usage() {
  cat <<EOF
Usage: $0 [--base-url URL] [--agent all|codex|claude|openclaw|custom] [--out DIR]

Examples:
  $0 --base-url http://localhost:8008 --agent all --out ./generated-agent-config
  $0 --agent codex
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base-url)
      BASE_URL="${2:-}"
      shift 2
      ;;
    --agent)
      AGENT="${2:-all}"
      shift 2
      ;;
    --out)
      OUT_DIR="${2:-./generated-agent-config}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "$BASE_URL" ]]; then
  echo "--base-url cannot be empty" >&2
  exit 2
fi
if [[ ! "$BASE_URL" =~ ^https?:// ]]; then
  echo "--base-url must start with http:// or https://" >&2
  exit 2
fi

if ! command -v curl >/dev/null 2>&1; then
  echo "curl is required" >&2
  exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required" >&2
  exit 1
fi

MANIFEST_URL="${BASE_URL%/}/agent-onboarding.json"
TMP_MANIFEST="$(mktemp)"
trap 'rm -f "$TMP_MANIFEST"' EXIT

curl -fsS \
  --retry 2 \
  --retry-connrefused \
  --connect-timeout 5 \
  --max-time 20 \
  "$MANIFEST_URL" > "$TMP_MANIFEST"
mkdir -p "$OUT_DIR"

jq '{edgeplane: .mcp_server}' "$TMP_MANIFEST" > "$OUT_DIR/edgeplane.mcp.json"

# Codex prefers TOML config entries with timeout controls.
jq -r '
  . as $m
  | "[mcp_servers.edgeplane]\n"
  + "command = \"" + $m.mcp_server.command + "\"\n"
  + "startup_timeout_sec = " + ($m.mcp_defaults.startup_timeout_sec|tostring) + "\n"
  + "tool_timeout_sec = " + ($m.mcp_defaults.tool_timeout_sec|tostring) + "\n"
  + "env = { "
  + ($m.mcp_server.env | to_entries | map(.key + " = \"" + (.value|tostring) + "\"") | join(", "))
  + " }\n"
' "$TMP_MANIFEST" > "$OUT_DIR/codex.mcp.toml"

# Claude/OpenClaw/Custom consume JSON snippets.
jq '{
  mcpServers: {
    edgeplane: {
      command: .mcp_server.command,
      env: .mcp_server.env
    }
  }
}' "$TMP_MANIFEST" > "$OUT_DIR/claude.mcp.json"

jq '{
  edgeplane: {
    transport: "acp-bridge",
    mcp_server: .mcp_server,
    endpoint_candidates: .mcp_defaults.endpoint_candidates,
    startup_timeout_sec: .mcp_defaults.startup_timeout_sec,
    tool_timeout_sec: .mcp_defaults.tool_timeout_sec
  }
}' "$TMP_MANIFEST" > "$OUT_DIR/openclaw.acp.json"

cp "$OUT_DIR/openclaw.acp.json" "$OUT_DIR/custom.acp.json"

write_agent_file() {
  local agent_name="$1"
  local manifest_key="$2"
  local file_name="$3"
  jq --arg key "$manifest_key" '{edgeplane: .agent_configs[$key].edgeplane}' "$TMP_MANIFEST" > "$OUT_DIR/$file_name"
  echo "wrote $OUT_DIR/$file_name"
}

case "$AGENT" in
  all)
    write_agent_file "codex" "codex" "codex.mcp.json"
    write_agent_file "claude" "claude_code" "claude-code.mcp.json"
    write_agent_file "openclaw" "openclaw_custom" "openclaw.mcp.json"
    write_agent_file "custom" "openclaw_custom" "custom.mcp.json"
    ;;
  codex)
    write_agent_file "codex" "codex" "codex.mcp.json"
    ;;
  claude)
    write_agent_file "claude" "claude_code" "claude-code.mcp.json"
    ;;
  openclaw)
    write_agent_file "openclaw" "openclaw_custom" "openclaw.mcp.json"
    ;;
  custom)
    write_agent_file "custom" "openclaw_custom" "custom.mcp.json"
    ;;
  *)
    echo "Invalid --agent value: $AGENT" >&2
    echo "Expected one of: all, codex, claude, openclaw, custom" >&2
    exit 2
    ;;
esac

echo "wrote $OUT_DIR/edgeplane.mcp.json"
echo "wrote $OUT_DIR/codex.mcp.toml"
echo "wrote $OUT_DIR/claude.mcp.json"
echo "wrote $OUT_DIR/openclaw.acp.json"
echo "wrote $OUT_DIR/custom.acp.json"
echo "source manifest: $MANIFEST_URL"
