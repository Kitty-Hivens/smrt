#!/usr/bin/env bash
# Build release binary locally, scp to VPS, restart systemd unit.
# Assumes SSH access via ~/.ssh/vps_hivens (override via KEY env).

set -euo pipefail

HOST=${HOST:-root@hivens.dev}
KEY=${KEY:-$HOME/.ssh/vps_hivens}
REMOTE_BIN=${REMOTE_BIN:-/usr/local/bin/smrt}

cd "$(dirname "$0")/.."

echo "==> cargo build --release"
cargo build --release

BINARY="target/release/smrt"
[[ -f "$BINARY" ]] || { echo "missing $BINARY after build" >&2; exit 1; }

SIZE=$(stat -c%s "$BINARY")
echo "==> binary built ($((SIZE / 1024 / 1024)) MB)"

echo "==> scp -> $HOST:$REMOTE_BIN.new"
scp -i "$KEY" "$BINARY" "$HOST:$REMOTE_BIN.new"

echo "==> swap + restart"
ssh -i "$KEY" "$HOST" "mv $REMOTE_BIN.new $REMOTE_BIN && chmod +x $REMOTE_BIN && systemctl restart smrt && sleep 1 && systemctl status smrt --no-pager -l | head -15"

echo "==> done"
