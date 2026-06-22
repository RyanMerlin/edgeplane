#!/bin/sh
set -eu

PREFIX="${EP_INSTALL_PREFIX:-/usr/local}"
BIN_SRC="${EP_BINARY_PATH:-$(command -v edgeplaned || true)}"
SERVICE_NAME="edgeplaned.service"
CONFIG_DIR="${EP_CONFIG_DIR:-/etc/edgeplane}"
ENV_FILE="${CONFIG_DIR}/edgeplaned.env"
SYSTEMD_DIR="${EP_SYSTEMD_DIR:-/etc/systemd/system}"

info() { echo "[INFO] $*" ; }
warn() { echo "[WARN] $*" >&2 ; }
fatal() { echo "[ERROR] $*" >&2 ; exit 1 ; }

if [ "$(id -u)" -ne 0 ]; then
  fatal "run as root"
fi

if [ -z "${BIN_SRC}" ] || [ ! -x "${BIN_SRC}" ]; then
  fatal "edgeplaned binary not found; set EP_BINARY_PATH to the built executable"
fi

install -d "${PREFIX}/bin" "${CONFIG_DIR}" "${SYSTEMD_DIR}"

# Dedicated non-root service account (idempotent).
id -u edgeplane >/dev/null 2>&1 || useradd --system --shell /usr/sbin/nologin \
  --home-dir /var/lib/edgeplane --create-home --user-group edgeplane

install -m 0755 "${BIN_SRC}" "${PREFIX}/bin/edgeplaned"

if [ ! -f "${ENV_FILE}" ]; then
  cat > "${ENV_FILE}" <<'EOF'
# EdgePlane node daemon settings
# Required:
EP_BASE_URL=https://edgeplane.example.com
EP_HOME=/var/lib/edgeplane
# Optional:
# EP_NODE_NAME=$(hostname -s)
# EP_NODE_TRUST_TIER=trusted
#
# To enroll this node after editing the above, run:
#   EP_HOME=/var/lib/edgeplane edgeplaned register --join-token <TOKEN> --endpoint $EP_BASE_URL
EOF
  chmod 0600 "${ENV_FILE}"
fi

install -m 0644 "$(dirname "$0")/systemd/edgeplaned.service" "${SYSTEMD_DIR}/${SERVICE_NAME}"
systemctl daemon-reload
info "installed edgeplaned and ${SERVICE_NAME}"
warn "edit ${ENV_FILE}, enroll via: EP_HOME=/var/lib/edgeplane edgeplaned register --join-token <TOKEN> --endpoint <EP_BASE_URL>"
warn "then enable with: systemctl enable --now edgeplaned.service"
