# Remote Backend (daemon)

This document describes the current remote backend implementation used by CodexMonitor.

The daemon runs backend logic in a separate process and exposes a line-delimited JSON-RPC protocol over TCP. The desktop app can connect to it in remote mode, and mobile flows use the same backend path.

An optional thin HTTP bridge is also available for simple remote submission and read-only inspection flows.

## Current Status

- Desktop app remote mode is wired into the app command surface.
- iOS remote usage is supported through the desktop-hosted daemon.
- Shared backend behavior should live in `src-tauri/src/shared/*`, with app and daemon acting as thin adapters.
- Some capabilities remain intentionally local-only and do not route through the daemon yet.

Related docs:

- `README.md`
- `docs/mobile-ios-tailscale-blueprint.md`
- `docs/codebase-map.md`
- `docs/daemon-rpc-runbook.md`
- `docs/daemon-service-api.md`

## Run

From the repo root:

```bash
cd src-tauri

# pick a strong token (or export CODEX_MONITOR_DAEMON_TOKEN)
TOKEN="change-me"

cargo run --bin codex_monitor_daemon -- \
  --listen 127.0.0.1:4732 \
  --http-listen 127.0.0.1:4733 \
  --data-dir "$HOME/.local/share/codex-monitor-daemon" \
  --token "$TOKEN"
```

Notes:

- In WSL2, Windows access usually requires binding to `0.0.0.0` depending on port forwarding setup.
- `--insecure-no-auth` exists for local development only.
- `--http-listen` is optional. When set, the HTTP bridge uses the same token as the TCP JSON-RPC daemon.

## Desktop Integration

Desktop CodexMonitor can operate in:

- Local mode: app commands execute in-process.
- Remote mode: app commands proxy to the daemon over TCP JSON-RPC.

The desktop app also includes daemon lifecycle helpers for TCP mode, including start, stop, status, and command preview flows used by the mobile/Tailscale setup.

## Current Local-Only Areas

These areas are still local/UI/platform-specific and should not be treated as daemon-parity gaps by default:

- Terminal session UI/runtime
- Dictation model lifecycle and microphone permission flows
- Tray integration
- Tailscale detection and managed daemon lifecycle controls
- Local file picker/export helpers such as arbitrary text-file import/export
- Window/menu/platform shell behavior

If one of these must work remotely, treat it as a deliberate design change rather than routine parity work.

## Command Boundary

Use this split when deciding whether a missing daemon method is a bug or an intentional local capability.

### Should Have Daemon Parity

These command families are part of the main remote workflow and should stay aligned across app and daemon:

- Workspace/worktree lifecycle:
  `list_workspaces`, `is_workspace_path_dir`, `add_workspace`, `add_workspace_from_git_url`,
  `add_clone`, `add_worktree`, `worktree_setup_status`, `worktree_setup_mark_ran`,
  `remove_workspace`, `remove_worktree`, `rename_worktree`, `rename_worktree_upstream`,
  `apply_worktree_changes`, `update_workspace_settings`, `set_workspace_runtime_codex_args`,
  `connect_workspace`, `list_workspace_files`, `read_workspace_file`, `open_workspace_in`,
  `get_open_app_icon`
- Codex/thread workflow:
  `get_config_model`, `codex_doctor`, `start_thread`, `send_user_message`, `turn_steer`,
  `turn_interrupt`, `start_review`, `respond_to_server_request`, `remember_approval_rule`,
  `generate_commit_message`, `generate_run_metadata`, `generate_agent_description`,
  `resume_thread`, `read_thread`, `thread_live_subscribe`, `thread_live_unsubscribe`,
  `fork_thread`, `list_threads`, `list_mcp_server_status`, `archive_thread`, `compact_thread`,
  `set_thread_name`, `collaboration_mode_list`, `model_list`, `experimental_feature_list`,
  `set_codex_feature_flag`, `account_rate_limits`, `account_read`, `codex_login`,
  `codex_login_cancel`, `skills_list`, `apps_list`
- Agent/config and prompt management:
  `file_read`, `file_write`, `get_agents_settings`, `set_agents_core_settings`,
  `create_agent`, `update_agent`, `delete_agent`, `read_agent_config_toml`,
  `write_agent_config_toml`, `prompts_list`, `prompts_create`, `prompts_update`,
  `prompts_delete`, `prompts_move`, `prompts_workspace_dir`, `prompts_global_dir`
- Git/GitHub workflow:
  `get_git_status`, `init_git_repo`, `create_github_repo`, `list_git_roots`,
  `get_git_diffs`, `get_git_log`, `get_git_commit_diff`, `get_git_remote`, `stage_git_file`,
  `stage_git_all`, `unstage_git_file`, `revert_git_file`, `revert_git_all`, `commit_git`,
  `push_git`, `pull_git`, `fetch_git`, `sync_git`, `get_github_issues`,
  `get_github_pull_requests`, `get_github_pull_request_diff`,
  `get_github_pull_request_comments`, `checkout_github_pull_request`,
  `list_git_branches`, `checkout_git_branch`, `create_git_branch`
- Shared read/query helpers:
  `local_usage_snapshot`

### App-Owned Coordination Commands

These commands may be visible through the daemon, but they are still owned by app/runtime coordination rather than shared domain behavior:

- `get_app_settings`
- `update_app_settings`
- `get_codex_config_path`
- `menu_set_accelerators`
- `is_macos_debug_build`
- `send_notification_fallback`

Treat changes here carefully. Some fields are local UI/runtime settings, while others control remote connectivity.

#### Keep Mirrored For Now

These still have practical value in remote flows today, but they are not clean shared-domain contracts yet:

- `get_app_settings`
  The frontend depends on this at bootstrap in every mode. It currently mixes local UI preferences with remote connectivity/runtime settings.
- `update_app_settings`
  Still needed because the same settings model is used to configure remote backend host/token/provider and some runtime startup behavior.

Preferred long-term direction:

- Split local shell/UI preferences from remote connection/backend settings.
- Keep only the remote-relevant subset mirrored through the daemon.

#### Prefer App-Only

These should generally be treated as app-shell commands even if a daemon helper exists today:

- `get_codex_config_path`
  Current usage is tied to revealing/opening a config path from the local desktop UI, not remote backend execution.
- `menu_set_accelerators`
  Pure desktop menu wiring.
- `is_macos_debug_build`
  Pure local build/runtime inspection.
- `send_notification_fallback`
  Pure local macOS debug-notification fallback.

Preferred long-term direction:

- Remove these from daemon parity expectations.
- Keep any remaining daemon implementations only if they are still required by current app wiring, not as a model for future shared surfaces.

### Intentionally Local Commands

These commands should remain local unless there is an explicit product decision to remote-enable them:

- Local text/image/file helpers:
  `write_text_file`, `read_text_file`, `list_text_files_in_directory`, `read_image_as_data_url`
- Terminal runtime:
  `terminal_open`, `terminal_write`, `terminal_resize`, `terminal_close`
- Dictation/runtime permission flow:
  `dictation_model_status`, `dictation_download_model`, `dictation_cancel_download`,
  `dictation_remove_model`, `dictation_start`, `dictation_request_permission`,
  `dictation_stop`, `dictation_cancel`
- Tray/platform shell:
  `set_tray_recent_threads`, `set_tray_session_usage`
- Tailscale and managed daemon lifecycle:
  `tailscale_status`, `tailscale_daemon_command_preview`, `tailscale_daemon_start`,
  `tailscale_daemon_stop`, `tailscale_daemon_status`
- Platform/runtime helpers:
  `is_mobile_runtime`, `app_build_type`

### Daemon-Internal Commands

These are transport/runtime methods and are not part of the frontend feature surface:

- `auth`
- `ping`
- `daemon_info`
- `daemon_shutdown`

## Local Verification

For HTTP bridge behavior, prefer daemon-only Rust tests instead of the full Tauri app build:

```bash
cd src-tauri
cargo test --no-default-features --bin codex_monitor_daemon http_health_returns_ok_with_valid_auth
cargo test --no-default-features --bin codex_monitor_daemon http_v1_task_create_and_get_round_trip
cargo test --no-default-features --bin codex_monitor_daemon task_status_transitions_from_app_server_events
```

This path avoids desktop WebKit/GTK runtime requirements and focuses verification on daemon TCP/HTTP behavior.

## Protocol

- One JSON object per line.
- Requests: `{"id": <number>, "method": "<string>", "params": <object|null>}`
- Responses: `{"id": <number>, "result": <any>}` or `{"id": <number>, "error": {"message": "<string>"}}`
- Events (server -> client notifications): `{"method":"app-server-event","params":{...}}`

### Auth Handshake

Required unless `--insecure-no-auth` is set.

First request must be:

```json
{"id": 1, "method": "auth", "params": {"token": "..." }}
```

## Quick Test With Netcat

```bash
printf '{\"id\":1,\"method\":\"auth\",\"params\":{\"token\":\"change-me\"}}\\n' | nc -w 1 127.0.0.1 4732
printf '{\"id\":2,\"method\":\"ping\"}\\n' | nc -w 1 127.0.0.1 4732
printf '{\"id\":3,\"method\":\"list_workspaces\",\"params\":{}}\\n' | nc -w 1 127.0.0.1 4732
```

## Optional HTTP Bridge

When `--http-listen` is enabled, the daemon exposes a minimal HTTP API intended for thin adapters such as curl scripts, shortcuts, bots, or webhook bridges.

Authentication:

- `Authorization: Bearer <token>`
- `X-Codex-Token: <token>`

Current routes:

- `GET /health`
- `GET /api/v1/health`
- `GET /api/workspaces`
- `GET /api/threads?workspaceId=<id>`
- `GET /api/thread?workspaceId=<id>&threadId=<id>`
- `POST /api/task/submit`
- `POST /api/v1/tasks`
- `GET /api/v1/tasks/{taskId}`
- `GET /api/v1/tasks/{taskId}/events`
- `GET /api/v1/events/threads?workspaceId=<id>&threadId=<id>`

Example:

```bash
curl -sS http://127.0.0.1:4733/api/task/submit \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "workspaceId": "workspace_123",
    "text": "修复登录重定向 bug，并提交代码",
    "accessMode": "full-access"
  }'
```

Successful response:

```json
{
  "accepted": true,
  "workspaceId": "workspace_123",
  "threadId": "thread_abc",
  "createdThread": true,
  "turnId": "turn_xyz"
}
```

Current `v1 tasks` note:

- `POST /api/v1/tasks` creates a service-side task record and submits work into Codex.
- `GET /api/v1/tasks/{taskId}` returns the current stored task metadata.
- `GET /api/v1/tasks/{taskId}/events` streams task updates plus filtered app-server events over SSE.
- `GET /api/v1/events/threads?...` streams filtered thread app-server events over SSE for web/mobile clients that already know the target thread.
- Task status now progresses in-memory through `accepted`, `running`, `completed`, and `failed`.
- Task records are now persisted to `service_tasks.json` under the daemon data dir.
- Task execution state is still reconstructed from live events only; active in-flight work is not recovered after a daemon restart and is marked failed on reload.

## Implemented RPC Surface

The daemon currently covers:

- Workspace/worktree lifecycle and workspace settings
- Codex thread lifecycle, live subscription, reviews, approvals, accounts, skills, apps, agent config, and feature flags
- Git and GitHub read/write operations used by the main app workflow
- Prompt CRUD/listing
- Shared file reads/writes for managed config surfaces
- Usage snapshot queries
- A small set of desktop coordination helpers such as `menu_set_accelerators`, `is_macos_debug_build`, and notification fallback

For exact method names, see:

- `src-tauri/src/bin/codex_monitor_daemon/rpc.rs`
- `src-tauri/src/bin/codex_monitor_daemon/rpc/*`
- `src-tauri/src/shared/git_rpc.rs`
- `src-tauri/src/shared/workspace_rpc.rs`

## Change Rules

When adding backend behavior that should work in both app and daemon:

1. Implement or move the domain logic into `src-tauri/src/shared/*`.
2. Wire the app adapter command.
3. Wire the daemon RPC method.
4. Keep request/response payload shapes aligned with `src/services/tauri.ts`.
5. Add or update tests around the touched shared core or RPC surface.
