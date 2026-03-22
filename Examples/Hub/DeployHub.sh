#!/bin/bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
REMOTE_HOST="${1:-}"
LOCAL_BINARY_PATH="$WORKSPACE_DIR/Source/openhanse-cli/Artefact/openhanse-cli-linux-x86_64"
REMOTE_BIN_DIR='.local/lib/openhanse-hub'
REMOTE_BIN_PATH="$REMOTE_BIN_DIR/openhanse-hub"

if [[ -z "$REMOTE_HOST" ]]; then
  echo "Usage: $0 <ssh-host>" >&2
  exit 1
fi

if [[ ! -f "$LOCAL_BINARY_PATH" ]]; then
  echo "Missing build artifact: $LOCAL_BINARY_PATH" >&2
  echo "Build it first with: cd $WORKSPACE_DIR/Source/openhanse-cli && ./BuildOpenHanseCli.sh" >&2
  exit 1
fi

ssh "$REMOTE_HOST" "systemctl --user stop openhanse-hub || true"
ssh "$REMOTE_HOST" "mkdir -p \$HOME/$REMOTE_BIN_DIR && rm -f \$HOME/$REMOTE_BIN_DIR/*"
scp "$LOCAL_BINARY_PATH" "$REMOTE_HOST:$REMOTE_BIN_PATH"
ssh "$REMOTE_HOST" "chmod 755 \$HOME/$REMOTE_BIN_PATH && systemctl --user start openhanse-hub"
