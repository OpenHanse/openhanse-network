#!/bin/bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
DEFAULT_BINARY_PATH="$WORKSPACE_DIR/Source/openhanse-cli/Artefact/openhanse-cli-macos-apple-silicon"
SERVER_URL="${OPENHANSE_TEST_SERVER_URL:-http://127.0.0.1:8080}"
BINARY_PATH="${OPENHANSE_TEST_BINARY:-$DEFAULT_BINARY_PATH}"
GATEWAY_HOST="${OPENHANSE_TEST_GATEWAY_HOST:-127.0.0.1}"
TERMINAL_APP="${OPENHANSE_TEST_APP:-auto}"
RUN_ROOT="$SCRIPT_DIR/.run"
TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
RUN_DIR="$RUN_ROOT/$TIMESTAMP"

usage() {
  cat <<EOF
Usage: $0

Starts a local three-terminal OpenHanse test run on macOS:
  1. hub
  2. gateway-a
  3. gateway-b

Optional environment variables:
  OPENHANSE_TEST_BINARY       Override the CLI binary path
  OPENHANSE_TEST_SERVER_URL   Override the local server URL
  OPENHANSE_TEST_GATEWAY_HOST Override the gateway bind host
  OPENHANSE_TEST_APP          auto, iterm, or terminal

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

has_iterm() {
  osascript -e 'if application "iTerm" exists then return "yes"' 2>/dev/null | grep -qx "yes"
}

resolve_terminal_app() {
  case "$TERMINAL_APP" in
    auto)
      if has_iterm; then
        echo "iterm"
      else
        echo "terminal"
      fi
      ;;
    iterm|terminal)
      echo "$TERMINAL_APP"
      ;;
    *)
      echo "Unsupported OPENHANSE_TEST_APP value: $TERMINAL_APP" >&2
      exit 1
      ;;
  esac
}

launch_iterm_layout() {
  local hub_command="$1"
  local gateway_a_command="$2"
  local gateway_b_command="$3"
  local hub_escaped gateway_a_escaped gateway_b_escaped

  hub_escaped="$(applescript_escape "$hub_command")"
  gateway_a_escaped="$(applescript_escape "$gateway_a_command")"
  gateway_b_escaped="$(applescript_escape "$gateway_b_command")"

  osascript <<EOF
tell application "iTerm"
  activate
  set new_window to (create window with default profile command "$hub_escaped")
  tell current session of current tab of new_window
    set bottom_session to (split horizontally with default profile command "$gateway_a_escaped")
  end tell
  tell bottom_session
    split vertically with default profile command "$gateway_b_escaped"
  end tell
end tell
EOF
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
APP_CHOICE="$(resolve_terminal_app)"
echo "Launcher: $APP_CHOICE"

if [[ "$APP_CHOICE" == "iterm" ]]; then
  launch_iterm_layout "$HUB_COMMAND" "$GATEWAY_A_COMMAND" "$GATEWAY_B_COMMAND"
else
  launch_terminal_window "hub" "$HUB_COMMAND"
  sleep 0.5
  launch_terminal_window "gateway-a" "$GATEWAY_A_COMMAND"
  sleep 0.5
  launch_terminal_window "gateway-b" "$GATEWAY_B_COMMAND"
fi

echo
if [[ "$APP_CHOICE" == "iterm" ]]; then
  echo "One iTerm window with a three-pane layout has been launched."
else
  echo "Three Terminal windows have been launched."
fi
echo "Logs:"
echo "  $HUB_LOG"
echo "  $GATEWAY_A_LOG"
echo "  $GATEWAY_B_LOG"
