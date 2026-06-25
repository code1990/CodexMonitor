# Daemon API Quickstart

This guide is the fastest way to verify that `codex_monitor_daemon` can accept a remote task and stream results back over SSE.

If you need the full route and payload reference, use `docs/daemon-service-api.md`.

## Goal

At the end of this guide you will have:

1. started the daemon HTTP service
2. verified health with `curl`
3. submitted one remote task
4. watched task events stream over SSE

## Before You Start

You need all of these to already be true:

- `codex_monitor_daemon` is built
- `codex` is installed and available in `PATH`
- you have a daemon data directory
- the target workspace already exists in daemon state
- the target workspace is already connected to a live Codex session

Important:

- the current HTTP `v1` API does not yet create/connect workspaces for you
- if the workspace is not already connected, task submission will fail with `workspace not connected`

## 1. Start the daemon

Example:

```bash
cd /root/CodexMonitor/CodexMonitor

TOKEN='replace-with-a-strong-token'
DATA_DIR="$HOME/.local/share/codex-monitor-daemon"

./src-tauri/target/release/codex_monitor_daemon \
  --listen 127.0.0.1:4732 \
  --http-listen 127.0.0.1:4733 \
  --data-dir "$DATA_DIR" \
  --token "$TOKEN"
```

If you have not built it yet:

```bash
./scripts/build-daemon.sh
```

## 2. Check health

In another terminal:

```bash
curl -H "Authorization: Bearer $TOKEN" \
  http://127.0.0.1:4733/api/v1/health
```

Expected result:

- HTTP `200 OK`
- JSON with `"ok": true`

## 3. Confirm the workspace is present

Legacy inspection route:

```bash
curl -H "Authorization: Bearer $TOKEN" \
  http://127.0.0.1:4733/api/workspaces
```

Find the workspace you want to target and note its `id`.

Example workspace id used below:

```text
ws-http
```

## 4. Submit a task

Use an existing thread if you already have one. If you omit `threadId`, the daemon will create one first.

Example request:

```bash
curl -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  http://127.0.0.1:4733/api/v1/tasks \
  -d '{
    "workspaceId": "ws-http",
    "text": "inspect the current repo and summarize the failing tests",
    "accessMode": "full-access"
  }'
```

Example response:

```json
{
  "task": {
    "taskId": "2d019f4f-3bd2-4a7f-8b8c-f10be2a8fa55",
    "status": "running",
    "workspaceId": "ws-http",
    "threadId": "thread-http",
    "turnId": "turn-1",
    "createdThread": true,
    "submittedAtMs": 1760000000000,
    "completedAtMs": null,
    "lastError": null
  }
}
```

Copy the returned `task.taskId`.

## 5. Stream task events

Use SSE to watch progress:

```bash
curl -N -H "Authorization: Bearer $TOKEN" \
  http://127.0.0.1:4733/api/v1/tasks/2d019f4f-3bd2-4a7f-8b8c-f10be2a8fa55/events
```

You should see:

- an initial `event: task`
- one or more `event: app-server-event`
- a final `event: task` with `status` equal to `completed` or `failed`

## 6. Poll final task state

If you want a non-streaming check:

```bash
curl -H "Authorization: Bearer $TOKEN" \
  http://127.0.0.1:4733/api/v1/tasks/2d019f4f-3bd2-4a7f-8b8c-f10be2a8fa55
```

Terminal task statuses today:

- `completed`
- `failed`

## 7. Optional: stream thread events instead of task events

If you want the raw app-server event flow for a workspace or thread:

```bash
curl -N -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:4733/api/v1/events/threads?workspaceId=ws-http&threadId=thread-http"
```

This is useful when:

- one client is following a long-lived thread
- you want lower-level event visibility than the task wrapper

## Common Failures

### `401 Unauthorized`

Cause:

- missing token
- wrong token

Fix:

- confirm `Authorization: Bearer <token>` matches the daemon start token

### `workspace not connected`

Cause:

- the workspace exists but has no live Codex session

Fix:

- connect the workspace first through the desktop app or daemon RPC path

### `task not found`

Cause:

- wrong `taskId`
- the task was never created successfully

Fix:

- resubmit the task and use the new returned `taskId`

## What To Read Next

- deployment and `systemd`: `docs/deploy-daemon.md`
- full HTTP/SSE reference: `docs/daemon-service-api.md`
- service product direction: `docs/remote-service-architecture.md`
