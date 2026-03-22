#!/bin/bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
DEFAULT_BINARY_PATH="$WORKSPACE_DIR/Source/openhanse-cli/Artefact/openhanse-cli-macos-apple-silicon"
SERVER_URL="${OPENHANSE_TEST_SERVER_URL:-http://127.0.0.1:8080}"
BINARY_PATH="${OPENHANSE_TEST_BINARY:-$DEFAULT_BINARY_PATH}"
GATEWAY_HOST="${OPENHANSE_TEST_GATEWAY_HOST:-127.0.0.1}"
RUN_ROOT="$SCRIPT_DIR/.run"
TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
RUN_DIR="$RUN_ROOT/$TIMESTAMP"

usage() {
  cat <<EOF
Usage: $0

Starts a local three-window OpenHanse test run on macOS using Apple Terminal only:
  1. hub
  2. gateway-a
  3. gateway-b

Optional environment variables:
  OPENHANSE_TEST_BINARY       Override the CLI binary path
  OPENHANSE_TEST_SERVER_URL   Override the local server URL
  OPENHANSE_TEST_GATEWAY_HOST Override the gateway bind host

Default binary:
  $DEFAULT_BINARY_PATH
EOF
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  usage
  exit 0
fi

if [[ ! -f "$BINARY_PATH" ]]; then
  echo "Missing build artifact: $BINARY_PATH" >&2
  echo "Build it first with: cd $WORKSPACE_DIR/Source/openhanse-cli && ./BuildOpenHanseCli.sh" >&2
  exit 1
fi

if [[ ! -x "$BINARY_PATH" ]]; then
  echo "Binary is not executable: $BINARY_PATH" >&2
  exit 1
fi

if ! command -v osascript >/dev/null 2>&1; then
  echo "This script requires macOS osascript to launch Terminal windows." >&2
  exit 1
fi

mkdir -p "$RUN_DIR"

applescript_escape() {
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  printf '%s' "$value"
}

build_terminal_command() {
  local label="$1"
  local logfile="$2"
  shift 2
  local args=("$@")
  local quoted_args=()
  local arg

  for arg in "${args[@]}"; do
    quoted_args+=("$(printf '%q' "$arg")")
  done

  printf 'clear; echo "OpenHanse %s"; echo "Log: %s"; echo; cd %q; %q %s 2>&1 | tee %q' \
    "$label" \
    "$logfile" \
    "$WORKSPACE_DIR" \
    "$BINARY_PATH" \
    "${quoted_args[*]}" \
    "$logfile"
}

launch_terminal_window() {
  local label="$1"
  local command="$2"
  local escaped

  escaped="$(applescript_escape "$command")"

  osascript <<EOF
tell application "Terminal"
  activate
  do script "$escaped"
end tell
EOF

  echo "Launched $label"
}

HUB_LOG="$RUN_DIR/hub.log"
GATEWAY_A_LOG="$RUN_DIR/gateway-a.log"
GATEWAY_B_LOG="$RUN_DIR/gateway-b.log"

HUB_COMMAND="$(build_terminal_command \
  "hub" \
  "$HUB_LOG" \
  --id hub \
  --peer-mode hub \
  --server "$SERVER_URL")"

GATEWAY_A_COMMAND="$(build_terminal_command \
  "gateway-a" \
  "$GATEWAY_A_LOG" \
  --id gateway-a \
  --target gateway-b \
  --peer-mode gateway \
  --server "$SERVER_URL" \
  --host "$GATEWAY_HOST")"

GATEWAY_B_COMMAND="$(build_terminal_command \
  "gateway-b" \
  "$GATEWAY_B_LOG" \
  --id gateway-b \
  --target gateway-a \
  --peer-mode gateway \
  --server "$SERVER_URL" \
  --host "$GATEWAY_HOST")"

echo "Run directory: $RUN_DIR"
echo "Server URL: $SERVER_URL"
echo "Gateway host: $GATEWAY_HOST"
echo "Binary: $BINARY_PATH"

launch_terminal_window "hub" "$HUB_COMMAND"
sleep 0.5
launch_terminal_window "gateway-a" "$GATEWAY_A_COMMAND"
sleep 0.5
launch_terminal_window "gateway-b" "$GATEWAY_B_COMMAND"

echo
echo "Three Terminal windows have been launched."
echo "Logs:"
echo "  $HUB_LOG"
echo "  $GATEWAY_A_LOG"
echo "  $GATEWAY_B_LOG"
