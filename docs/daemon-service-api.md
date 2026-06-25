# Daemon Service API

This document describes the current HTTP and SSE service surface exposed by `codex_monitor_daemon`.

Scope of this document:

- what routes exist today
- exact auth model used today
- task lifecycle and streaming behavior
- current limits you need to account for as a client

Related docs:

- `docs/daemon-api-quickstart.md`
- `docs/deploy-daemon.md`
- `docs/remote-service-architecture.md`
- `REMOTE_BACKEND_POC.md`

## Status

This is an early `v1` service surface, not yet a fully stabilized public platform API.

What is already usable:

- health checks
- create task
- query task state
- subscribe to task SSE
- subscribe to thread SSE

What is not yet provided as a clean HTTP API:

- workspace connect/create flows
- thread create/list/read under `/api/v1/*`
- approval response endpoints
- scoped tokens or per-client credentials

## Base URL

When the daemon is started with `--http-listen 127.0.0.1:4733`, the base URL is:

```text
http://127.0.0.1:4733
```

In production, put a reverse proxy in front and expose HTTPS there.

## Authentication

If the daemon was started with `--token <value>`, every HTTP request must include one of:

```http
Authorization: Bearer <token>
```

or:

```http
X-Codex-Token: <token>
```

If auth is missing or wrong, the daemon returns:

```json
{
  "error": "unauthorized"
}
```

with HTTP status `401 Unauthorized`.

`--insecure-no-auth` disables auth and should only be used for local development.

## Response Shape

Current `v1` routes do not yet use a single universal envelope. In practice:

- health returns `{ "ok": true, ... }`
- task create/read returns `{ "task": { ... } }`
- errors return `{ "error": "<message>" }`

Do not assume future routes will keep every legacy detail unchanged.

## Task Model

`POST /api/v1/tasks` creates a service-side task record persisted in:

```text
<data-dir>/service_tasks.json
```

Current task fields:

```json
{
  "taskId": "2d019f4f-3bd2-4a7f-8b8c-f10be2a8fa55",
  "status": "running",
  "workspaceId": "ws-http",
  "threadId": "thread-http",
  "turnId": "turn-1",
  "createdThread": false,
  "submittedAtMs": 1760000000000,
  "completedAtMs": null,
  "lastError": null
}
```

Current statuses:

- `accepted`
- `running`
- `completed`
- `failed`

Status transitions are driven from daemon-side app-server events:

- `turn/started` -> `running`
- `turn/completed` -> `completed`
- `error` or `turn/error` -> `failed`

If the daemon restarts before a task finishes, persisted in-flight tasks are reloaded as `failed` with:

```json
{
  "lastError": "daemon restarted before task completion"
}
```

## Important Current Limitation

The HTTP task API is not a full workspace bootstrap API.

Before submitting a task:

- the workspace must already exist in daemon state
- the workspace must already be connected to a live Codex session

Today, that setup typically happens through:

- the desktop app
- daemon JSON-RPC methods
- existing workspace persistence under the daemon data directory

The HTTP `v1` API does not yet auto-connect a workspace for you.

## Endpoints

### `GET /api/v1/health`

Returns daemon health and basic daemon metadata.

Example:

```bash
curl -H "Authorization: Bearer $TOKEN" \
  http://127.0.0.1:4733/api/v1/health
```

Example response:

```json
{
  "ok": true,
  "daemon": {
    "name": "codex-monitor-daemon",
    "version": "<app-version>",
    "pid": 12345,
    "mode": "tcp",
    "binaryPath": "/opt/codex-monitor-daemon/bin/codex_monitor_daemon"
  },
  "http": true,
  "version": "v1"
}
```

Notes:

- legacy `GET /health` also exists
- `binaryPath` may be `null` in some environments

### `POST /api/v1/tasks`

Creates a task record and immediately forwards the request into the existing Codex thread flow.

Request body:

```json
{
  "workspaceId": "ws-http",
  "threadId": "thread-http",
  "text": "inspect the current repo and summarize the failing tests",
  "model": null,
  "effort": null,
  "serviceTier": null,
  "accessMode": "full-access",
  "images": null,
  "appMentions": null,
  "collaborationMode": null
}
```

Field notes:

- `workspaceId`: required
- `text`: required
- `threadId`: optional; when omitted, the daemon creates a new thread first
- `accessMode`: forwarded into the existing message-send path
- `model`, `effort`, `serviceTier`, `images`, `appMentions`, `collaborationMode`: optional passthrough fields

Example:

```bash
curl -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  http://127.0.0.1:4733/api/v1/tasks \
  -d '{
    "workspaceId": "ws-http",
    "threadId": "thread-http",
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
    "createdThread": false,
    "submittedAtMs": 1760000000000,
    "completedAtMs": null,
    "lastError": null
  }
}
```

Current behavior details:

- HTTP status is currently `200 OK`, not `201 Created`
- a task is inserted as `accepted`, then usually promoted to `running` immediately
- if no `threadId` is passed, `createdThread` becomes `true`

### `GET /api/v1/tasks/{taskId}`

Returns the latest persisted state for a task.

Example:

```bash
curl -H "Authorization: Bearer $TOKEN" \
  http://127.0.0.1:4733/api/v1/tasks/2d019f4f-3bd2-4a7f-8b8c-f10be2a8fa55
```

Example response:

```json
{
  "task": {
    "taskId": "2d019f4f-3bd2-4a7f-8b8c-f10be2a8fa55",
    "status": "completed",
    "workspaceId": "ws-http",
    "threadId": "thread-http",
    "turnId": "turn-1",
    "createdThread": false,
    "submittedAtMs": 1760000000000,
    "completedAtMs": 1760000004321,
    "lastError": null
  }
}
```

If the task does not exist:

```json
{
  "error": "task not found"
}
```

with HTTP status `404 Not Found`.

### `GET /api/v1/tasks/{taskId}/events`

Streams task updates and related app-server events over SSE.

Headers:

```http
Content-Type: text/event-stream
Cache-Control: no-cache
Connection: keep-alive
```

Example:

```bash
curl -N -H "Authorization: Bearer $TOKEN" \
  http://127.0.0.1:4733/api/v1/tasks/2d019f4f-3bd2-4a7f-8b8c-f10be2a8fa55/events
```

Event types currently emitted:

- `task`
- `app-server-event`

Initial event:

```text
event: task
data: {"task":{"taskId":"...","status":"running","workspaceId":"ws-http","threadId":"thread-http","turnId":"turn-1","createdThread":false,"submittedAtMs":1760000000000,"completedAtMs":null,"lastError":null}}
```

Then, while the task is active, you may receive:

```text
event: app-server-event
data: {"workspace_id":"ws-http","message":{"method":"turn/started","params":{"threadId":"thread-http","turnId":"turn-1"}}}
```

and later:

```text
event: task
data: {"task":{"taskId":"...","status":"completed","workspaceId":"ws-http","threadId":"thread-http","turnId":"turn-1","createdThread":false,"submittedAtMs":1760000000000,"completedAtMs":1760000004321,"lastError":null}}
```

Current stream behavior:

- the stream sends the latest task snapshot first
- keep-alive comments are sent every 15 seconds when idle
- the stream closes after the task reaches a terminal state
- there is no cursor or `Last-Event-ID` resume support yet

### `GET /api/v1/events/threads?workspaceId=...&threadId=...`

Streams raw app-server events for a workspace, optionally filtered to a specific thread.

Required query:

- `workspaceId`

Optional query:

- `threadId`

Example:

```bash
curl -N -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:4733/api/v1/events/threads?workspaceId=ws-http&threadId=thread-http"
```

Initial event:

```text
event: stream
data: {"workspaceId":"ws-http","threadId":"thread-http","kind":"thread-events"}
```

Subsequent events:

```text
event: app-server-event
data: {"workspace_id":"ws-http","message":{"method":"turn/started","params":{"threadId":"thread-http","turnId":"turn-1"}}}
```

Current behavior details:

- when `threadId` is omitted, all thread events for that workspace are streamed
- filtering is based on app-server event `threadId`
- keep-alive comments are sent every 15 seconds when idle

## Legacy HTTP Routes

These still exist and are useful for compatibility or quick inspection:

- `GET /health`
- `GET /api/workspaces`
- `GET /api/threads?workspaceId=...`
- `GET /api/thread?workspaceId=...&threadId=...`
- `POST /api/task/submit`

`POST /api/task/submit` differs from `POST /api/v1/tasks`:

- it submits work immediately
- it does not create a persisted `taskId`
- it returns a direct acceptance payload such as:

```json
{
  "accepted": true,
  "workspaceId": "ws-http",
  "threadId": "thread-http",
  "createdThread": false,
  "turnId": "turn-1"
}
```

Prefer `/api/v1/tasks` for new integrations.

## Error Handling

Current common HTTP error patterns:

- `401 Unauthorized`
- `404 Not Found`
- `400 Bad Request`

Typical `400` cases:

- invalid JSON body
- missing `workspaceId`
- missing `taskId`
- workspace not connected
- lower-level thread/message submission errors

Error body shape:

```json
{
  "error": "workspace not connected"
}
```

## Suggested Client Flow

For browser, mobile, or automation clients, the recommended sequence today is:

1. Call `GET /api/v1/health`
2. Ensure the target workspace already exists and is connected
3. Call `POST /api/v1/tasks`
4. Read the returned `task.taskId`
5. Subscribe to `GET /api/v1/tasks/{taskId}/events`
6. Optionally poll `GET /api/v1/tasks/{taskId}` for fallback state checks

## Known Gaps

Current service API gaps worth planning around:

- no workspace bootstrap route in `v1`
- no approval response route in `v1`
- no idempotency key support
- no pagination or list-tasks route
- no scoped auth tokens
- no SSE replay or cursor resume support
- no documented compatibility guarantees across major refactors yet

## Local Verification

Daemon-focused Rust tests for this API live in:

- `src-tauri/src/bin/codex_monitor_daemon.rs`

Useful commands:

```bash
cd src-tauri
cargo test --no-default-features --bin codex_monitor_daemon http_health_returns_ok_with_valid_auth
cargo test --no-default-features --bin codex_monitor_daemon http_v1_task_create_and_get_round_trip
cargo test --no-default-features --bin codex_monitor_daemon task_status_transitions_from_app_server_events
```
