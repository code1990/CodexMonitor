# Daemon RPC Runbook

This document covers the daemon management surface that sits beside the HTTP API:

- `codex_monitor_daemonctl` for lifecycle and status
- TCP JSON-RPC for direct daemon calls

Use this document when you need to:

- start or stop the daemon without the desktop UI
- debug remote backend connectivity
- script against the raw daemon protocol

If you need the browser/mobile-facing HTTP and SSE API, use `docs/daemon-service-api.md`.

## Surfaces

CodexMonitor daemon currently exposes two different remote surfaces:

1. TCP JSON-RPC on `--listen`
2. Optional HTTP/SSE on `--http-listen`

This document is about the first one.

## `codex_monitor_daemonctl`

The control CLI is the supported management entrypoint for daemon lifecycle operations.

Source:

- `src-tauri/src/bin/codex_monitor_daemonctl.rs`

Supported commands today:

- `start`
- `stop`
- `status`
- `command-preview`

## Basic Usage

Show status:

```bash
cd /root/CodexMonitor/CodexMonitor/src-tauri
./target/debug/codex_monitor_daemonctl status
```

Start daemon:

```bash
./target/debug/codex_monitor_daemonctl start
```

Stop daemon:

```bash
./target/debug/codex_monitor_daemonctl stop
```

Print the equivalent daemon launch command:

```bash
./target/debug/codex_monitor_daemonctl command-preview
```

## How `daemonctl` Resolves Defaults

If you do not pass explicit flags, the CLI derives defaults from the app data directory and its `settings.json`.

Key behavior:

- `--data-dir` points to the app data directory
- listen address is derived from settings when present
- token is derived from settings when present
- if no data dir is provided, platform default app data location is used

The usage text in the binary currently defines the main flags as:

- `--listen <addr>`
- `--token <token>`
- `--data-dir <path>`
- `--daemon-path <path>`
- `--insecure-no-auth`
- `--json`

## Recommended CLI Patterns

### Explicit local dev invocation

```bash
./target/debug/codex_monitor_daemonctl status \
  --listen 127.0.0.1:4732 \
  --token 'replace-with-a-strong-token'
```

### Machine-readable status

```bash
./target/debug/codex_monitor_daemonctl status --json
```

### Explicit daemon binary path

```bash
./target/debug/codex_monitor_daemonctl start \
  --daemon-path /opt/codex-monitor-daemon/bin/codex_monitor_daemon \
  --data-dir /var/lib/codex-monitor-daemon \
  --listen 127.0.0.1:4732 \
  --token 'replace-with-a-strong-token'
```

### Local development without auth

```bash
./target/debug/codex_monitor_daemonctl start --insecure-no-auth
```

Only use `--insecure-no-auth` for local development.

## What `status` Means

`daemonctl status` probes the TCP daemon and classifies the endpoint into one of these broad states:

- daemon reachable and authenticated
- daemon reachable but auth failed
- port reachable but not a recognized daemon
- nothing reachable on that address

Internally it uses:

- `ping`
- optional `auth`
- `daemon_info`

That means `status` is more reliable than just checking whether the port is open.

## What `start` Does

`start` is not a blind spawn wrapper.

Current behavior includes:

- refusing to start without a token unless `--insecure-no-auth` is set
- probing the target port first
- detecting whether an existing process is already the expected daemon
- reusing a healthy compatible daemon when possible
- attempting restart logic when a mismatched managed daemon is detected

This matters operationally because `start` is trying to avoid stomping on unrelated processes using the same port.

## What `stop` Does

`stop` first tries to shut the daemon down through its own control path.

If that fails, forced-stop behavior is guarded:

- it is only attempted when daemon ownership can be verified
- it refuses to kill arbitrary non-daemon processes on the same port

This is the intended safety model. Use `daemonctl stop`, not ad hoc `kill -9`, unless you are already doing manual incident recovery.

## `command-preview`

`command-preview` is useful when you want:

- a copyable launch command for system scripts
- to confirm the resolved data dir, token mode, and listen address
- to compare desktop-managed settings with a server deployment layout

It can also emit JSON:

```bash
./target/debug/codex_monitor_daemonctl command-preview --json
```

## TCP JSON-RPC Protocol

The raw daemon protocol is line-delimited JSON over TCP.

Shape:

- request: `{"id":1,"method":"ping","params":{}}`
- success: `{"id":1,"result":{...}}`
- error: `{"id":1,"error":{"message":"..."}}`
- event: `{"method":"app-server-event","params":{...}}`

Important:

- one JSON object per line
- request `id` is required if you want a response
- app-server events are pushed asynchronously after subscription flows

## Authentication Handshake

Unless the daemon was started with `--insecure-no-auth`, authenticate first.

Example request:

```json
{"id":1,"method":"auth","params":{"token":"replace-with-a-strong-token"}}
```

Then verify:

```json
{"id":2,"method":"ping","params":{}}
```

## Minimal Manual Session

Using `nc`:

```bash
nc 127.0.0.1 4732
```

Then send lines like:

```json
{"id":1,"method":"auth","params":{"token":"replace-with-a-strong-token"}}
{"id":2,"method":"ping","params":{}}
{"id":3,"method":"daemon_info","params":{}}
{"id":4,"method":"list_workspaces","params":{}}
```

Expected examples:

```json
{"id":2,"result":{"ok":true}}
```

```json
{"id":3,"result":{"name":"codex-monitor-daemon","version":"<app-version>","pid":12345,"mode":"tcp","binaryPath":"/path/to/codex_monitor_daemon"}}
```

## Useful Low-Level Methods

### Runtime control methods

These are daemon-internal but useful for diagnostics:

- `auth`
- `ping`
- `daemon_info`
- `daemon_shutdown`

### Common operational methods

These are often enough to verify a remote backend end-to-end:

- `list_workspaces`
- `connect_workspace`
- `start_thread`
- `send_user_message`
- `list_threads`
- `read_thread`
- `thread_live_subscribe`
- `thread_live_unsubscribe`

## Example: Verify a Workspace

List workspaces:

```json
{"id":10,"method":"list_workspaces","params":{}}
```

Connect one workspace:

```json
{"id":11,"method":"connect_workspace","params":{"id":"ws-http"}}
```

Start a thread:

```json
{"id":12,"method":"start_thread","params":{"workspaceId":"ws-http"}}
```

The exact returned payload depends on the method, but this flow is enough to prove:

- daemon auth works
- workspace routing works
- Codex session spawn/connect works

## Example: Submit a Message Over RPC

Once you have a workspace id and thread id:

```json
{"id":13,"method":"send_user_message","params":{"workspaceId":"ws-http","threadId":"thread-http","text":"inspect the current repo and summarize the failing tests","accessMode":"full-access"}}
```

This is the raw RPC equivalent of the higher-level HTTP task flow.

## Example: Live Event Subscription

Subscribe to a thread:

```json
{"id":14,"method":"thread_live_subscribe","params":{"workspaceId":"ws-http","threadId":"thread-http"}}
```

After that, the daemon can push notifications like:

```json
{"method":"app-server-event","params":{"workspace_id":"ws-http","message":{"method":"turn/started","params":{"threadId":"thread-http","turnId":"turn-1"}}}}
```

When finished:

```json
{"id":15,"method":"thread_live_unsubscribe","params":{"workspaceId":"ws-http","threadId":"thread-http"}}
```

## When To Use RPC Instead Of HTTP

Prefer TCP JSON-RPC when:

- you need parity with the desktop app remote mode
- you want access to daemon methods not yet exposed over HTTP
- you are building an internal tool or admin/debug utility

Prefer HTTP/SSE when:

- the client is browser/mobile oriented
- you want simple proxy-friendly transport
- you are integrating with webhooks, bots, or service-side adapters

## Common Failure Cases

### `unauthorized` or `invalid token`

Cause:

- missing or wrong token

Fix:

- send `auth` first
- verify the token matches daemon startup settings

### Port is open but `daemonctl status` is not healthy

Cause:

- another process is using the port
- the daemon is running but version/ownership/auth checks failed

Fix:

- use `daemonctl status --json`
- inspect `last_error`
- avoid killing the process blindly unless you have verified ownership

### `workspace not connected`

Cause:

- workspace exists but no live Codex session is attached yet

Fix:

- call `connect_workspace`
- or connect through the desktop app first

## Recommended Document Flow

- daemon lifecycle and raw protocol: `docs/daemon-rpc-runbook.md`
- HTTP/SSE reference: `docs/daemon-service-api.md`
- curl-based smoke test: `docs/daemon-api-quickstart.md`
- deployment and `systemd`: `docs/deploy-daemon.md`
