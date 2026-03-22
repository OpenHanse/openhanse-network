# OpenHanse / Examples / Test / RustRunner

This folder contains a local desktop test runner for OpenHanse CLI processes inside one Rust window.

The current runner is:

- `openhanse-test-pty`: uses a PTY so the child behaves more like it is running in a real terminal

Both apps use the same three-pane layout:

- hub on top
- `gateway-a` bottom-left
- `gateway-b` bottom-right

## Run

From this directory:

cargo run --bin openhanse-test-pty
```

The apps default to this CLI artifact:

```bash
openhanse-network/Source/openhanse-cli/Artefact/openhanse-cli-macos-apple-silicon
```

Make sure that artifact exists first.

## Current Behavior

The PTY version is closer to a real terminal session:

- one terminal-like combined stream
- the child sees a PTY instead of plain pipes
- useful for checking prompt behavior and other interactive differences

This is still intentionally MVP-level, but it is now the preferred direction for the embedded local test runner.
