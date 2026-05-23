#!/bin/sh
set -eu

PREFIX="${EP_INSTALL_PREFIX:-/usr/local}"
BIN_SRC="${EP_BINARY_PATH:-$(command -v edgeplane || true)}"
SERVICE_NAME="edgeplane-node.service"
CONFIG_DIR="${EP_CONFIG_DIR:-/etc/edgeplane}"
ENV_FILE="${CONFIG_DIR}/${SERVICE_NAME}.env"
SYSTEMD_DIR="${EP_SYSTEMD_DIR:-/etc/systemd/system}"

info() { echo "[INFO] $*" ; }
warn() { echo "[WARN] $*" >&2 ; }
fatal() { echo "[ERROR] $*" >&2 ; exit 1 ; }

if [ "$(id -u)" -ne 0 ]; then
  fatal "run as root"
fi

if [ -z "${BIN_SRC}" ] || [ ! -x "${BIN_SRC}" ]; then
  fatal "edgeplane binary not found; set EP_BINARY_PATH to the built executable"
fi

install -d "${PREFIX}/bin" "${CONFIG_DIR}" "${SYSTEMD_DIR}"
install -m 0755 "${BIN_SRC}" "${PREFIX}/bin/edgeplane"
ln -fsn edgeplane "${PREFIX}/bin/ep"

if [ ! -f "${ENV_FILE}" ]; then
  cat > "${ENV_FILE}" <<'EOF'
# Edgeplane node settings
# Required:
# EP_BASE_URL=https://edgeplane.example.com
# EP_NODE_BOOTSTRAP_TOKEN=...
# Optional:
# EP_NODE_NAME=$(hostname -s)
# EP_NODE_HOSTNAME=$(hostname -f)
# EP_NODE_TRUST_TIER=trusted
# EP_NODE_POLL_SECONDS=30
# EP_NODE_HEARTBEAT_SECONDS=15
# EP_NODE_UPGRADE_MANIFEST_URL=https://edgeplane.example.com/releases/latest.json
# EP_HOME=/var/lib/edgeplane
EOF
  chmod 0600 "${ENV_FILE}"
fi

install -m 0644 "$(dirname "$0")/systemd/edgeplane-node.service" "${SYSTEMD_DIR}/${SERVICE_NAME}"
systemctl daemon-reload
info "installed edgeplane and ${SERVICE_NAME}"
warn "populate ${ENV_FILE}, then run: systemctl enable --now ${SERVICE_NAME}"
