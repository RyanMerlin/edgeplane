#!/usr/bin/env sh
# mc-precompact.sh — run by Claude Code's PreCompact hook.
# Outputs the current MC context state to stdout so it's preserved in the
# archival summary before compaction discards it from the context window.
set -eu

CONTEXT_FILE="${MC_INSTANCE_HOME:-$HOME/.missioncontrol}/mc/context.json"

if [ -f "$CONTEXT_FILE" ]; then
    echo "[MC Context — Pre-Compact Snapshot]"
    # Pretty-print key fields if jq is available, otherwise dump raw JSON.
    if command -v jq >/dev/null 2>&1; then
        jq -r '
            "Domain: \(.active_domain_id // "none")",
            "Mission: \(.active_mission_id // "none")",
            "Profile: \(.active_profile // "none")",
            "Last sync: \(.last_sync_at // "unknown")"
        ' "$CONTEXT_FILE" 2>/dev/null || cat "$CONTEXT_FILE"
    else
        cat "$CONTEXT_FILE"
    fi
else
    echo "[MC Context — no context.json found at ${CONTEXT_FILE}]"
fi

exit 0
