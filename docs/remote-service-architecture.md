# Remote Service Architecture

This document defines how CodexMonitor can evolve from a desktop companion into a standalone middleware service that fronts a server-hosted Codex runtime.

## Goal

Target deployment model:

1. Codex CLI / app-server runs on a server as the execution brain.
2. CodexMonitor daemon runs as a long-lived middleware service.
3. Web pages, mobile apps, or other clients call CodexMonitor over HTTP/WebSocket/SSE.
4. CodexMonitor manages workspace/session/thread routing, forwards requests into Codex, and returns results/events to clients.

This makes CodexMonitor closer to an application server than a desktop-only control plane.

Related docs:

- `docs/deploy-daemon.md`
- `docs/daemon-service-api.md`
- `docs/hula-im-research-friends-model.md`
- `docs/pytdx2-product-reference.md`
- `REMOTE_BACKEND_POC.md`

## Current Starting Point

Existing building blocks already in the repo:

- Standalone daemon binary:
  `src-tauri/src/bin/codex_monitor_daemon.rs`
- Standalone daemon control CLI:
  `src-tauri/src/bin/codex_monitor_daemonctl.rs`
- Shared backend cores:
  `src-tauri/src/shared/*`
- TCP JSON-RPC transport already used by app remote mode
- Thin HTTP bridge already implemented in the daemon:
  - `GET /health`
  - `GET /api/workspaces`
  - `GET /api/threads?workspaceId=...`
  - `GET /api/thread?workspaceId=...&threadId=...`
  - `POST /api/task/submit`

What exists today is enough to prove the direction, but not enough yet to serve as a general multi-client middleware API.

Current incremental progress in this repo:

- `POST /api/v1/tasks`
- `GET /api/v1/tasks/{taskId}`
- `GET /api/v1/tasks/{taskId}/events`
- `GET /api/v1/events/threads?workspaceId=...&threadId=...`

These routes provide an initial service-side task resource with file-backed task persistence, lifecycle tracking, and SSE streaming. Tasks that were still in progress when the daemon stopped are surfaced as failed after restart rather than left indefinitely running.

## Desired Service Shape

CodexMonitor should support a dedicated "service mode" with these properties:

- Runs headless without desktop UI
- Can be started independently like Tomcat/systemd services
- Exposes stable external APIs for non-Tauri clients
- Manages authentication, routing, workspaces, threads, and event fanout
- Supports both request/response and streaming updates
- Treats desktop app as just another client, not the owner of the backend

## Service Responsibilities

In this model, CodexMonitor becomes responsible for:

- Workspace registry and workspace connection lifecycle
- Thread creation, resume, read, archive, naming, and compact flows
- Request submission into Codex sessions
- Streaming Codex events back to external clients
- API authentication and client isolation
- Persistent service state for workspaces/settings/session metadata
- Optional policy enforcement such as access mode allowlists or workspace ACLs

CodexMonitor should not become responsible for:

- Replacing Codex execution semantics
- Browser/mobile UI state management
- Desktop shell concerns such as tray/menu/window behavior

## Required API Modes

To serve web/mobile clients well, request/response alone is not enough.

### 1. Synchronous HTTP API

Use for:

- Health
- Workspace list/read operations
- Thread list/read operations
- Submit task / create thread / enqueue message
- Simple admin operations

Suggested direction:

- Keep `/api/task/submit`
- Add a versioned namespace such as `/api/v1/*`
- Normalize response envelopes

### 2. Streaming API

Use for:

- Incremental assistant output
- Tool events
- Approval requests
- Turn lifecycle state
- Thread live updates

Recommended first choice:

- SSE for browser/mobile simplicity

Recommended later option:

- WebSocket for bidirectional streaming and lower-latency multiplexing

Suggested endpoints:

- `GET /api/v1/tasks/{taskId}/events`
- Later: `GET /api/v1/threads/events?workspaceId=...&threadId=...`

This should map to the existing app-server event flow already emitted inside the daemon.

### 3. Async Job API

Use when clients do not want to hold an open connection.

Suggested flow:

1. `POST /api/v1/tasks`
2. Server returns `taskId`, `workspaceId`, `threadId`
3. Client polls `GET /api/v1/tasks/{taskId}` or subscribes to `GET /api/v1/tasks/{taskId}/events`

This is the cleanest model for mobile push, webhook relays, and third-party integrations.

## Recommended Architecture

Do not build a second backend beside the daemon.

Preferred evolution path:

1. Keep `shared/*` as the source of truth.
2. Keep the daemon as the only service host.
3. Expand the daemon's HTTP layer into a real external API surface.
4. Optionally add SSE/WebSocket support in the same daemon process.
5. Let desktop Tauri app consume the same service contracts where practical.

In short:

- CodexMonitor daemon becomes the server product.
- Tauri desktop becomes one client of that server.

## Service Layers

Recommended layering inside the repo:

1. Shared domain core:
   `src-tauri/src/shared/*`
2. Service application layer:
   daemon state + service-oriented orchestration in
   `src-tauri/src/bin/codex_monitor_daemon.rs`
3. External transport layer:
   HTTP + SSE/WebSocket endpoints
4. Desktop adapter:
   current Tauri commands and remote client bridge

The main rule is:

- External HTTP/WebSocket handlers should call service/domain methods, not duplicate business logic.

## Main Gaps Today

### Gap 1: HTTP surface is too thin

Current HTTP bridge only supports a few endpoints.

Needed next:

- thread create/read/list actions in a clean REST shape
- message submit API beyond the current thin wrapper
- review/approval endpoints where needed
- workspace connect/read helpers
- stable error codes and response envelopes

### Gap 2: No external streaming contract

The daemon already has internal event fanout, but external web/mobile clients cannot yet subscribe to it through a browser-friendly transport.

Needed next:

- SSE endpoint for thread/app-server events
- event serialization contract
- reconnect semantics using cursor or last-event-id

### Gap 3: Authentication is too coarse

Current model is basically a shared bearer token.

Needed next:

- service tokens with scopes
- optional client identity
- audit-friendly request metadata

### Gap 4: No service-oriented task abstraction

Current `/api/task/submit` is a thin message submit helper.

Needed next:

- explicit `taskId`
- task status model
- task-to-thread mapping
- retry/idempotency keys

### Gap 5: Desktop settings and service settings are mixed

Current settings contain both local UI preferences and remote backend connection settings.

Needed next:

- split service config from desktop UI config
- define a daemon-owned config model for headless deployment

## Suggested External API v1

Minimal first service API:

- `GET /api/v1/health`
- `GET /api/v1/workspaces`
- `POST /api/v1/workspaces/connect`
- `GET /api/v1/threads?workspaceId=...`
- `GET /api/v1/threads/{threadId}?workspaceId=...`
- `POST /api/v1/threads`
- `POST /api/v1/messages`
- `POST /api/v1/reviews`
- `POST /api/v1/approvals/respond`
- `GET /api/v1/events/threads?workspaceId=...&threadId=...`

Optional admin/service endpoints:

- `GET /api/v1/service/info`
- `GET /api/v1/service/config`
- `POST /api/v1/service/config`

## Recommended Transport Choice

For your described use case, the pragmatic order is:

1. HTTP REST + SSE
2. Add WebSocket only if bidirectional interactive control becomes necessary

Why:

- Browser and mobile support for SSE is simpler
- Easier to debug through proxies
- Good fit for "submit request, stream assistant output"

## Deployment Model

The daemon should support running like a normal service:

- `systemd`
- Docker container
- supervisor/PM2 style long-lived process
- manual binary startup

Suggested deployment shape:

- `codex_monitor_daemon` as the primary server binary
- reverse proxy in front if public exposure is needed
- TLS termination outside the daemon initially

## Security Baseline

Minimum baseline before public or semi-public exposure:

- bearer token auth
- HTTPS via reverse proxy
- request size limits
- workspace allowlist
- per-token rate limiting
- audit logs for task submission and approval actions

Recommended later:

- tenant/client separation
- RBAC scopes
- webhook signatures or signed callbacks

## Implementation Phases

### Phase 1: Stabilize service positioning

- Treat daemon as the canonical standalone service runtime
- Keep expanding current HTTP bridge instead of starting a parallel server
- Split service config from desktop-only config in design

### Phase 2: Build usable external API

- Add `/api/v1/*` routes
- Add normalized JSON envelopes
- Add explicit task resource model

### Phase 3: Add streaming

- Add SSE endpoint for thread/app-server events
- Add event subscription registry
- Map internal app-server events to client-safe public events

### Phase 4: Harden for production

- token scopes
- audit logs
- rate limiting
- idempotency keys
- reverse-proxy deployment docs

## Concrete Recommendation

For this repo, the correct implementation path is:

- Do not create a separate Tomcat-like server product outside CodexMonitor.
- Promote `codex_monitor_daemon` into the standalone middleware server.
- Expand its HTTP layer into a first-class external API.
- Add SSE as the first external streaming transport.

That reuses existing domain logic, keeps app/daemon parity intact, and avoids building two backends that will drift.

## Immediate Next Steps

Best next implementation sequence:

1. Add a versioned HTTP API module layout under the daemon.
2. Add `POST /api/v1/tasks` with `taskId`.
3. Add `GET /api/v1/tasks/{taskId}`.
4. Add `GET /api/v1/events/threads` as SSE.
5. Introduce a daemon-owned service config file separate from desktop UI settings.

If this direction is accepted, the next code change should be to add the v1 service routes and the initial task resource model on top of the existing HTTP bridge.
