#!/bin/sh
set -eu

PREFIX="${MC_INSTALL_PREFIX:-/usr/local}"
BIN_SRC="${MC_BINARY_PATH:-$(command -v mc || true)}"
SERVICE_NAME="mc-node.service"
CONFIG_DIR="${MC_CONFIG_DIR:-/etc/missioncontrol}"
ENV_FILE="${CONFIG_DIR}/${SERVICE_NAME}.env"
SYSTEMD_DIR="${MC_SYSTEMD_DIR:-/etc/systemd/system}"

info() { echo "[INFO] $*" ; }
warn() { echo "[WARN] $*" >&2 ; }
fatal() { echo "[ERROR] $*" >&2 ; exit 1 ; }

if [ "$(id -u)" -ne 0 ]; then
  fatal "run as root"
fi

if [ -z "${BIN_SRC}" ] || [ ! -x "${BIN_SRC}" ]; then
  fatal "mc binary not found; set MC_BINARY_PATH to the built executable"
fi

install -d "${PREFIX}/bin" "${CONFIG_DIR}" "${SYSTEMD_DIR}"
install -m 0755 "${BIN_SRC}" "${PREFIX}/bin/mc"

if [ ! -f "${ENV_FILE}" ]; then
  cat > "${ENV_FILE}" <<'EOF'
# MissionControl node settings
# Required:
# MC_BASE_URL=https://missioncontrol.example.com
# MC_NODE_BOOTSTRAP_TOKEN=...
# Optional:
# MC_NODE_NAME=$(hostname -s)
# MC_NODE_HOSTNAME=$(hostname -f)
# MC_NODE_TRUST_TIER=trusted
# MC_NODE_POLL_SECONDS=30
# MC_NODE_HEARTBEAT_SECONDS=15
# MC_NODE_UPGRADE_MANIFEST_URL=https://missioncontrol.example.com/releases/latest.json
# MC_HOME=/var/lib/missioncontrol
EOF
  chmod 0600 "${ENV_FILE}"
fi

install -m 0644 "$(dirname "$0")/systemd/mc-node.service" "${SYSTEMD_DIR}/${SERVICE_NAME}"
systemctl daemon-reload
info "installed mc and ${SERVICE_NAME}"
warn "populate ${ENV_FILE}, then run: systemctl enable --now ${SERVICE_NAME}"
