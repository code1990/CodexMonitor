# Daemon Deployment

This runbook describes how to build and deploy `codex_monitor_daemon` as a standalone backend service.

Related API reference:

- `docs/daemon-api-quickstart.md`
- `docs/daemon-rpc-runbook.md`
- `docs/daemon-service-api.md`

## Goal

Target shape:

- `codex` runs on the server as the execution engine.
- `codex_monitor_daemon` runs as a long-lived middleware process.
- Web/mobile/third-party clients call the daemon over HTTP and SSE.
- A reverse proxy handles HTTPS and public exposure.

The daemon is the deployable backend product. The desktop UI is not required on the server.

For a `pytdx2`-style always-on backend, the deployment unit is the daemon itself, not the Tauri desktop application.

## Build

Build only the headless daemon binaries:

```bash
cd /root/CodexMonitor/CodexMonitor
./scripts/build-daemon.sh
```

That compiles:

- `src-tauri/target/release/codex_monitor_daemon`
- `src-tauri/target/release/codex_monitor_daemonctl`

Equivalent direct cargo command:

```bash
cd src-tauri
cargo build --release --no-default-features \
  --bin codex_monitor_daemon \
  --bin codex_monitor_daemonctl
```

Debug build:

```bash
PROFILE=debug ./scripts/build-daemon.sh
```

Package a standalone deployable archive:

```bash
./scripts/package-daemon.sh
```

That produces a tarball under `dist/`, containing:

- `bin/codex_monitor_daemon`
- `bin/codex_monitor_daemonctl`
- `deploy/codex-monitor-daemon.service`
- `deploy/nginx-codex-monitor-daemon.conf`
- deployment docs

Why `--no-default-features`:

- it avoids pulling in the Tauri desktop stack
- it keeps daemon builds independent from GTK/WebKit desktop requirements
- it is the correct build path for server deployment

## Server Prerequisites

Minimum recommended packages:

- `git`
- `curl`
- `pkg-config`
- `openssl-devel`
- Rust toolchain for on-server builds, or prebuilt binaries copied from CI
- `codex` CLI installed and available in `PATH`

Recommended runtime layout:

```text
/opt/codex-monitor-daemon/
  bin/
    codex_monitor_daemon
    codex_monitor_daemonctl
/etc/codex-monitor-daemon/
  codex-monitor-daemon.env
/var/lib/codex-monitor-daemon/
  workspaces.json
  settings.json
  service_tasks.json
```

Recommended release flow:

1. Build on CI or a dedicated build host with `./scripts/package-daemon.sh`
2. Copy the generated `dist/*.tar.gz` to the target server
3. Extract to `/opt/codex-monitor-daemon` or your release directory
4. Install the `systemd` unit and environment file
5. Put `nginx` or `caddy` in front for HTTPS and access control

## Manual Start

Example:

```bash
/opt/codex-monitor-daemon/bin/codex_monitor_daemon \
  --listen 127.0.0.1:4732 \
  --http-listen 127.0.0.1:4733 \
  --data-dir /var/lib/codex-monitor-daemon \
  --token 'replace-with-a-strong-token'
```

Notes:

- `--listen` is the internal TCP JSON-RPC port.
- `--http-listen` is the HTTP/SSE port for web/mobile clients.
- `--data-dir` stores workspaces, settings, and persisted task state.
- `--token` is currently the daemon's shared bearer token.

## systemd

Template unit file:

- [deploy/codex-monitor-daemon.service](/root/CodexMonitor/CodexMonitor/deploy/codex-monitor-daemon.service)

Suggested install:

```bash
sudo useradd --system --home /opt/codex-monitor-daemon --shell /sbin/nologin codex || true
sudo mkdir -p /opt/codex-monitor-daemon/bin
sudo mkdir -p /etc/codex-monitor-daemon
sudo mkdir -p /var/lib/codex-monitor-daemon
sudo cp src-tauri/target/release/codex_monitor_daemon /opt/codex-monitor-daemon/bin/
sudo cp src-tauri/target/release/codex_monitor_daemonctl /opt/codex-monitor-daemon/bin/
sudo cp deploy/codex-monitor-daemon.service /etc/systemd/system/
sudo chown -R codex:codex /opt/codex-monitor-daemon /var/lib/codex-monitor-daemon
```

Example environment file:

```bash
sudo tee /etc/codex-monitor-daemon/codex-monitor-daemon.env >/dev/null <<'EOF'
CODEX_MONITOR_TOKEN=replace-with-a-strong-token
CODEX_MONITOR_TCP_LISTEN=127.0.0.1:4732
CODEX_MONITOR_HTTP_LISTEN=127.0.0.1:4733
CODEX_MONITOR_DATA_DIR=/var/lib/codex-monitor-daemon
EOF
```

Enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable codex-monitor-daemon
sudo systemctl start codex-monitor-daemon
sudo systemctl status codex-monitor-daemon
```

## Reverse Proxy

Template config:

- [deploy/nginx-codex-monitor-daemon.conf](/root/CodexMonitor/CodexMonitor/deploy/nginx-codex-monitor-daemon.conf)

Recommended topology:

- daemon listens on `127.0.0.1:4733`
- `nginx` or `caddy` terminates HTTPS
- public clients only hit the reverse proxy

Important proxy settings:

- disable proxy buffering for SSE
- increase read timeout for long-lived streams
- keep bearer token forwarding intact if clients send it directly

## Current External API

Useful service endpoints today:

- `GET /api/v1/health`
- `POST /api/v1/tasks`
- `GET /api/v1/tasks/{taskId}`
- `GET /api/v1/tasks/{taskId}/events`
- `GET /api/v1/events/threads?workspaceId=...&threadId=...`

These are enough for a basic server-hosted request/response plus streaming workflow.
For request/response shapes and SSE event examples, see `docs/daemon-service-api.md`.

## Verification

Health check:

```bash
curl -H "Authorization: Bearer ${CODEX_MONITOR_TOKEN}" \
  http://127.0.0.1:4733/api/v1/health
```

Create a task:

```bash
curl -H "Authorization: Bearer ${CODEX_MONITOR_TOKEN}" \
  -H "Content-Type: application/json" \
  http://127.0.0.1:4733/api/v1/tasks \
  -d '{
    "workspaceId": "workspace_123",
    "threadId": "thread_123",
    "text": "inspect the current repo and summarize the failing tests",
    "accessMode": "full-access"
  }'
```

Daemon-only test path:

```bash
cd src-tauri
cargo test --no-default-features --bin codex_monitor_daemon
```

Recommended pre-release verification:

```bash
cd src-tauri
cargo test --no-default-features --bin codex_monitor_daemon http_health_returns_ok_with_valid_auth
cargo test --no-default-features --bin codex_monitor_daemon http_v1_task_create_and_get_round_trip
cargo test --no-default-features --bin codex_monitor_daemon task_status_transitions_from_app_server_events
```

## Operational Notes

- `service_tasks.json` persists task metadata across daemon restarts.
- In-flight tasks are not resumed after a daemon restart; they are marked failed on reload.
- Keep the daemon behind a reverse proxy if it will be reachable outside a trusted network.
- Current auth is a shared token; do not expose it directly to untrusted browser code without an intermediate backend you control.

## Next Hardening Steps

- move token into a secret manager or protected env file
- add request logging and audit logs
- add rate limiting at the proxy layer
- split daemon service config from desktop-derived settings
- package the daemon build in CI artifacts or container images
