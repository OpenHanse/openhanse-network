# OpenHanse / Examples / Hub

This example shows how to run a simple OpenHanse hub on a Linux machine.

## Build

Build the Linux binary from the main repository:

```bash
cd openhanse-network/Source/openhanse-cli
./BuildOpenHanseCli.sh
```

This produces artifacts such as:

- `openhanse-network/Source/openhanse-cli/Artefact/openhanse-cli-linux-x86_64`
- `openhanse-network/Source/openhanse-cli/Artefact/openhanse-cli-linux-aarch64`

This deploy example uploads that binary and runs it in `--peer-mode hub`.

## Upload

Use the deploy script in this example to upload only the binary:

```bash
cd openhanse-network/Examples/Hub
./DeployHub.sh user@example.com
```

The binary is uploaded to `~/.local/lib/openhanse-hub/openhanse-hub`.

## Run With systemd

The `systemd` setup is intentionally done manually by the server owner.

An example user service file is available at [Linux/systemd/user/openhanse-hub.service](/Volumes/Git/GitHub/OpenHanse/openhanse-network/Examples/Hub/Linux/systemd/user/openhanse-hub.service). Copy or adapt it into:

```bash
~/.config/systemd/user/openhanse-hub.service
```

Then reload and start it:

```bash
systemctl --user daemon-reload
systemctl --user enable openhanse-hub.service
systemctl --user restart openhanse-hub.service
```

If the service should keep running after logout, enable linger for that user:

```bash
loginctl enable-linger <user>
```

This location is intentionally user-local and fits a `systemctl --user` service without requiring `sudo`.

## Required Runtime Settings

The deployed hub currently uses these defaults:

- peer mode `hub`
- peer id `hub`
- server URL `http://0.0.0.0:8080`
- UDP discovery on `0.0.0.0:3478`

Make sure the host firewall allows TCP `8080` and UDP `3478` if the hub should be reachable from outside the machine.
