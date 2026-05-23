#!/usr/bin/env sh
# mc-recompact-context.sh — run by Claude Code's SessionStart (compact) hook.
# Re-injects the MC context into Claude's window immediately after compaction.
# Output from this script is injected as context by Claude Code.
set -eu

CONTEXT_FILE="${MC_INSTANCE_HOME:-$HOME/.missioncontrol}/mc/context.json"

echo "[MC Context — Post-Compact Re-injection]"

if [ -f "$CONTEXT_FILE" ]; then
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
    echo "No MC context file found — run 'missioncontrol set_active_domain' to set your domain."
fi

exit 0
