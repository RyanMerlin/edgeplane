#!/bin/sh
# install-edgeplane-node.sh — one-command system-mode node enrollment for EdgePlane.
# Downloads the edgeplaned binary from GitHub releases, verifies checksum, creates
# the edgeplane system user, installs a hardened systemd unit, enrolls the node,
# and enables the service.
#
# Usage:
#   bash <(curl -fsSL https://raw.githubusercontent.com/RyanMerlin/edgeplane/main/scripts/install-edgeplane-node.sh) \
#     --endpoint https://edgeplane.example.com \
#     --join-token <TOKEN> \
#     [--node-name <NAME>] \
#     [--version <VERSION>]

set -eu

ENDPOINT=""
JOIN_TOKEN=""
NODE_NAME=""
VERSION=""

info()  { printf '[INFO]  %s\n' "$*"; }
warn()  { printf '[WARN]  %s\n' "$*" >&2; }
fatal() { printf '[ERROR] %s\n' "$*" >&2; exit 1; }

# ── Parse flags ──────────────────────────────────────────────────────────────

while [ $# -gt 0 ]; do
  case "$1" in
    --endpoint)   ENDPOINT="$2";   shift 2 ;;
    --join-token) JOIN_TOKEN="$2"; shift 2 ;;
    --node-name)  NODE_NAME="$2";  shift 2 ;;
    --version)    VERSION="$2";    shift 2 ;;
    *) fatal "unknown flag: $1" ;;
  esac
done

[ -n "$ENDPOINT" ]   || fatal "--endpoint is required"
[ -n "$JOIN_TOKEN" ] || fatal "--join-token is required"
[ "$(id -u)" -eq 0 ] || fatal "run as root (sudo)"

if [ -z "$NODE_NAME" ]; then
  NODE_NAME="$(hostname -s)"
fi

# ── Enforce HTTPS (dev/local loopback exempt) ────────────────────────────────

case "$ENDPOINT" in
  https://*) ;;
  http://localhost*|http://127.0.0.1*) warn "using non-HTTPS endpoint — dev/local only" ;;
  *) fatal "--endpoint must use https:// (got: $ENDPOINT)" ;;
esac

# ── Dependency check ─────────────────────────────────────────────────────────

for cmd in curl systemctl useradd sha256sum; do
  command -v "$cmd" >/dev/null 2>&1 || fatal "required command not found: $cmd"
done

# ── Resolve version ──────────────────────────────────────────────────────────

if [ -z "$VERSION" ]; then
  VERSION="$(curl -fsSL https://raw.githubusercontent.com/RyanMerlin/edgeplane/main/VERSION)"
  [ -n "$VERSION" ] || fatal "could not resolve latest version from VERSION file"
  info "resolved latest version: $VERSION"
fi

# ── Detect architecture ──────────────────────────────────────────────────────

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64)  BINARY_NAME="edgeplaned-linux-x86_64" ;;
  aarch64) BINARY_NAME="edgeplaned-linux-aarch64" ;;
  *) fatal "unsupported architecture: $ARCH" ;;
esac

BASE_URL="https://github.com/RyanMerlin/edgeplane/releases/download/v${VERSION}"
BINARY_URL="${BASE_URL}/${BINARY_NAME}"
CHECKSUM_URL="${BASE_URL}/checksums.txt"

# ── Download + verify ────────────────────────────────────────────────────────

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

info "downloading $BINARY_NAME v${VERSION}"
curl -fsSL --output "$TMPDIR/$BINARY_NAME" "$BINARY_URL"

info "verifying checksum"
curl -fsSL --output "$TMPDIR/checksums.txt" "$CHECKSUM_URL"

# Extract the expected hash for our binary and verify
EXPECTED="$(grep " $BINARY_NAME$" "$TMPDIR/checksums.txt" | awk '{print $1}')"
[ -n "$EXPECTED" ] || fatal "checksum entry for $BINARY_NAME not found in checksums.txt"

ACTUAL="$(sha256sum "$TMPDIR/$BINARY_NAME" | awk '{print $1}')"
[ "$ACTUAL" = "$EXPECTED" ] || fatal "checksum mismatch: expected=$EXPECTED actual=$ACTUAL"
info "checksum verified"

# ── Create system user ───────────────────────────────────────────────────────

id -u edgeplane >/dev/null 2>&1 || \
  useradd --system --shell /usr/sbin/nologin \
    --home-dir /var/lib/edgeplane --create-home --user-group edgeplane

install -d -o edgeplane -g edgeplane /var/lib/edgeplane

# ── Install binary ───────────────────────────────────────────────────────────

install -m 0755 "$TMPDIR/$BINARY_NAME" /usr/local/bin/edgeplaned
info "installed /usr/local/bin/edgeplaned"

# ── Write env file (only if absent) ─────────────────────────────────────────

CONFIG_DIR=/etc/edgeplane
install -d "$CONFIG_DIR"

if [ ! -f "$CONFIG_DIR/edgeplaned.env" ]; then
  cat > "$CONFIG_DIR/edgeplaned.env" <<ENVEOF
EP_BASE_URL=${ENDPOINT}
EP_HOME=/var/lib/edgeplane
ENVEOF
  info "wrote $CONFIG_DIR/edgeplaned.env"
fi
# Always repair ownership and mode — corrects pre-existing bad perms on re-run.
chown root:root "$CONFIG_DIR/edgeplaned.env"
chmod 0600 "$CONFIG_DIR/edgeplaned.env"

# ── Install systemd unit (embedded verbatim from crates/edgeplane/systemd/) ──

cat > /etc/systemd/system/edgeplaned.service <<'UNITEOF'
[Unit]
Description=EdgePlane node daemon
Documentation=https://github.com/RyanMerlin/edgeplane
Wants=network-online.target
After=network-online.target

[Service]
Type=simple
User=edgeplane
Group=edgeplane
StateDirectory=edgeplane
WorkingDirectory=/var/lib/edgeplane
Environment=HOME=/var/lib/edgeplane
Environment=EP_HOME=/var/lib/edgeplane
EnvironmentFile=-/etc/edgeplane/edgeplaned.env
ExecStart=/usr/local/bin/edgeplaned run
Restart=on-failure
RestartSec=5s
TimeoutStopSec=30
KillMode=mixed
KillSignal=SIGTERM
StandardOutput=journal
StandardError=journal
SyslogIdentifier=edgeplaned
LimitNOFILE=1048576
# --- calibrated hardening (preserves the unprivileged-userns sandbox jail) ---
# Do NOT add RestrictNamespaces, PrivateUsers, or a restrictive SystemCallFilter:
# each breaks the jail's unshare(CLONE_NEWUSER|...). See Axis 2 spec §5.1.
# The future jail-activation axis will add Delegate=yes + ProtectControlGroups=no.
NoNewPrivileges=yes
ProtectSystem=strict
ReadWritePaths=/var/lib/edgeplane
ProtectHome=yes
PrivateTmp=yes
ProtectControlGroups=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
RestrictSUIDSGID=yes

[Install]
WantedBy=multi-user.target
UNITEOF

systemctl daemon-reload
info "installed /etc/systemd/system/edgeplaned.service"

# ── Enroll node ───────────────────────────────────────────────────────────────

# NOTE: --join-token is briefly visible in /proc/<pid>/cmdline during enrollment
# and may appear in sudo audit logs. Tokens are single-use and short-lived;
# rotate immediately if exposure is a concern. Long-term fix: EP_JOIN_TOKEN env
# var support in edgeplaned register (tracked separately).
info "enrolling node '$NODE_NAME'"
sudo -u edgeplane env EP_HOME=/var/lib/edgeplane \
  edgeplaned register \
    --join-token "$JOIN_TOKEN" \
    --endpoint "$ENDPOINT" \
    --node-name "$NODE_NAME"

# ── Enable + start service ───────────────────────────────────────────────────

systemctl enable --now edgeplaned.service
info "edgeplaned.service enabled and started"
info "node '$NODE_NAME' enrolled and running"
