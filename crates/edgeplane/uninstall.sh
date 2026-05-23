#!/bin/sh
set -eu

SERVICE_NAME="edgeplane-node.service"
SYSTEMD_DIR="${EP_SYSTEMD_DIR:-/etc/systemd/system}"
PREFIX="${EP_INSTALL_PREFIX:-/usr/local}"

if [ "$(id -u)" -ne 0 ]; then
  echo "[ERROR] run as root" >&2
  exit 1
fi

systemctl disable --now "${SERVICE_NAME}" >/dev/null 2>&1 || true
rm -f "${SYSTEMD_DIR}/${SERVICE_NAME}"
systemctl daemon-reload
rm -f "${PREFIX}/bin/edgeplane"
echo "[INFO] uninstalled ${SERVICE_NAME} and ${PREFIX}/bin/edgeplane"
