# Daemon 对话接口 Curl 测试说明

本文档用于验证 `codex_monitor_daemon` 当前分支上的 HTTP 对话接口。

适用场景：

1. 验证 daemon HTTP 服务是否可用
2. 验证鉴权是否生效
3. 验证 conversation 相关路由是否可达
4. 验证在 workspace 已存在且已连接时，完整对话流程是否可用

相关文档：

- `docs/daemon-service-api.md`
- `docs/daemon-api-quickstart.md`
- `develop_01.md` 不是运行时规范，当前以代码实现和本测试文档为准

## 使用说明

这份文档建议按以下顺序使用：

1. 先执行 Test 1 到 Test 8
2. 确认 workspace 已存在且已连接后，再执行 Test 9 到 Test 13
3. 测试时建议保留两个终端：
   一个看 daemon 日志
   一个执行 `curl`

实操建议：

- 使用独立的临时 `DATA_DIR`，不要和正式环境混用
- 不要复用旧的 `WORKSPACE_ID`，每次测试前都重新调用 `/api/workspaces`
- 业务侧主键以 `conversationId` 为准
- `threadId` 主要用于底层排查和关联分析
- 如果要观察状态变化，先启动 SSE，再从另一个终端发送 follow-up 消息

出现以下情况时应先停止继续测试并处理前置问题：

- Test 1 或 Test 2 失败：先排查 daemon 是否启动成功、token 是否正确
- Test 3 返回空 workspace：先准备 workspace
- Test 9 返回 `code: workspace_not_connected`：先通过 RPC 或桌面端建立 workspace 会话

## 指令场景

`title`、`requirement` 和 follow-up 的 `text` 不建议随意写长篇内容。

如果只是验证链路是否打通，优先使用简短、明确、低歧义的指令。

### 场景 A：链路冒烟测试

用途：

- 验证 `conversation/start` 是否能成功受理请求
- 验证是否能创建 thread 和 task

建议请求内容：

```json
{
  "title": "HTTP 对话冒烟测试",
  "requirement": "阅读当前仓库，并输出一句简洁摘要。"
}
```

特点：

- 文案短
- 歧义少
- 足以触发一次正常回复

### 场景 B：仓库理解测试

用途：

- 验证 agent 是否能读取当前 workspace
- 验证回复是否基于实际仓库上下文

建议请求内容：

```json
{
  "title": "仓库结构总结",
  "requirement": "阅读当前仓库，识别主要运行组件，并用 5 行总结。"
}
```

适合：

- 确认 workspace 路径是否正确
- 快速确认 Codex 是否具备读仓能力

### 场景 C：缺陷修复受理测试

用途：

- 模拟真实业务开发请求
- 验证 daemon 是否能承载较真实的工程类对话

建议请求内容：

```json
{
  "title": "修复 HTTP 对话路由问题",
  "requirement": "检查 daemon 的 HTTP conversation 路由，定位一个可复现问题，说明原因，并给出最小修复方案。"
}
```

适合：

- 想测试更接近真实开发的请求
- 后续还要继续追问同一会话

### 场景 D：继续追问 / 续聊

用途：

- 验证同一个 `conversationId` 能否继续发送消息
- 验证 follow-up 是否会创建新的 task 并复用原 thread

建议 follow-up 文本：

```text
继续，并把最终答案压缩成 3 行。
```

其他可用示例：

```text
只列出涉及的文件以及每个文件的修改原因。
```

```text
重新审视上一个回答，并指出你最不确定的一个风险点。
```

### 场景 E：失败路径验证

用途：

- 验证接口是否按预期安全失败

建议覆盖：

- 缺少鉴权
- 缺少必填字段
- 不存在的 `conversationId`
- 存在但未连接的 `workspaceId`

价值：

- 快速确认 HTTP 契约在异常情况下是否可预测

## 前置条件

需要满足：

- 已构建当前分支上的 daemon 二进制
- `codex` 已安装并可在 `PATH` 中访问
- 有可用的 daemon 数据目录
- daemon 状态中至少已有一个 workspace

如果要执行完整正向对话测试，还需要：

- 目标 workspace 已连接到 live Codex session

## 建议环境变量

```bash
export TOKEN='test-token-http'
export HOST='127.0.0.1'
export HTTP_PORT='4733'
export BASE_URL="http://${HOST}:${HTTP_PORT}"
export DATA_DIR='/tmp/codex-monitor-http-tests'
```

## 启动 Daemon

先构建当前分支 daemon：

```bash
cd /root/project/CodexMonitor
PROFILE=debug ./scripts/build-daemon.sh
```

启动方式：

```bash
/root/project/CodexMonitor/src-tauri/target/debug/codex_monitor_daemon \
  --listen 127.0.0.1:4732 \
  --http-listen "${HOST}:${HTTP_PORT}" \
  --data-dir "${DATA_DIR}" \
  --token "${TOKEN}"
```

保持该进程在独立终端中持续运行。

## Test 1：未带 Token 的健康检查

```bash
curl -i "${BASE_URL}/api/v1/health"
```

预期结果：

- HTTP 状态为 `401 Unauthorized`
- 返回体包含 `"error":"unauthorized"`

## Test 2：带 Token 的健康检查

```bash
curl -sS -H "Authorization: Bearer ${TOKEN}" \
  "${BASE_URL}/api/v1/health"
```

预期结果：

- JSON 中包含 `"ok":true`
- JSON 中包含 `"http":true`
- JSON 中包含 `"version":"v1"`

## Test 3：查询 Workspace 列表

```bash
curl -sS -H "Authorization: Bearer ${TOKEN}" \
  "${BASE_URL}/api/workspaces"
```

预期结果：

- HTTP `200`
- 返回 JSON 对象，且包含 `workspaces`

如果 `workspaces` 为空，当前不能继续做正向 conversation 测试。

## Test 4：缺少字段的 Start Conversation 请求

```bash
curl -i -H "Authorization: Bearer ${TOKEN}" \
  -H "Content-Type: application/json" \
  "${BASE_URL}/api/v1/conversations/start" \
  -d '{}'
```

预期结果：

- HTTP `400 Bad Request`

## Test 5：查询 Conversation 列表

```bash
curl -sS -H "Authorization: Bearer ${TOKEN}" \
  "${BASE_URL}/api/v1/conversations"
```

预期结果：

- HTTP `200`
- 返回 JSON 对象，且包含 `ok: true`
- 返回 JSON 对象，且包含 `data.items`

## Test 6：读取不存在的 Conversation

```bash
curl -i -H "Authorization: Bearer ${TOKEN}" \
  "${BASE_URL}/api/v1/conversations/not-found"
```

预期结果：

- HTTP `404 Not Found`
- 返回体包含 `"code":"conversation_not_found"`

## Test 7：读取不存在 Conversation 的消息

```bash
curl -i -H "Authorization: Bearer ${TOKEN}" \
  "${BASE_URL}/api/v1/conversations/not-found/messages"
```

预期结果：

- HTTP `404 Not Found`

## Test 8：向不存在的 Conversation 发送 Follow-up

```bash
curl -i -H "Authorization: Bearer ${TOKEN}" \
  -H "Content-Type: application/json" \
  "${BASE_URL}/api/v1/conversations/not-found/messages" \
  -d '{"text":"hello"}'
```

预期结果：

- HTTP `404 Not Found`

## Test 9：正向启动 Conversation

执行前，先从 `GET /api/workspaces` 结果中选择一个当前可用且已连接的 `workspaceId`：

```bash
export WORKSPACE_ID='replace-with-real-workspace-id'
```

执行：

```bash
curl -sS -H "Authorization: Bearer ${TOKEN}" \
  -H "Content-Type: application/json" \
  "${BASE_URL}/api/v1/conversations/start" \
  -d "{
    \"workspaceId\": \"${WORKSPACE_ID}\",
    \"title\": \"HTTP conversation smoke test\",
    \"requirement\": \"Inspect the repository and report one concise summary.\",
    \"operator\": \"curl-test\",
    \"accessMode\": \"full-access\"
  }"
```

成功时预期结果：

- HTTP `202 Accepted`
- JSON 中包含 `ok: true`
- JSON 中包含 `data.conversation.conversationId`
- JSON 中包含 `data.conversation.workspaceId`
- JSON 中包含 `data.conversation.threadId`
- JSON 中包含 `data.task.taskId`

如果 workspace 未连接，当前分支上的典型失败返回为：

```json
{
  "ok": false,
  "error": {
    "code": "workspace_not_connected",
    "message": "workspace not connected"
  }
}
```

## Test 10：读取 Conversation 详情

```bash
export CONVERSATION_ID='replace-with-returned-conversation-id'

curl -sS -H "Authorization: Bearer ${TOKEN}" \
  "${BASE_URL}/api/v1/conversations/${CONVERSATION_ID}"
```

预期结果：

- JSON 中包含 `ok: true`
- JSON 中包含 `data.conversation`
- `data.conversation.conversationId` 与请求值一致
- `data.conversation.status` 会随着执行过程变化，例如：
  `accepted`、`streaming`、`completed`

## Test 11：读取 Conversation 消息

```bash
curl -sS -H "Authorization: Bearer ${TOKEN}" \
  "${BASE_URL}/api/v1/conversations/${CONVERSATION_ID}/messages"
```

预期结果：

- JSON 中包含 `ok: true`
- JSON 中包含 `data.conversation`
- JSON 中包含 `data.messages`

## Test 12：发送 Follow-up 消息

```bash
curl -sS -H "Authorization: Bearer ${TOKEN}" \
  -H "Content-Type: application/json" \
  "${BASE_URL}/api/v1/conversations/${CONVERSATION_ID}/messages" \
  -d '{"text":"Continue and provide the final answer in 3 lines.", "accessMode":"full-access"}'
```

预期结果：

- HTTP `202 Accepted`
- JSON 中包含 `ok: true`
- JSON 中包含 `data.task.taskId`
- JSON 中包含 `data.task.threadId`

## Test 13：订阅 Conversation SSE

```bash
curl -N -H "Authorization: Bearer ${TOKEN}" \
  "${BASE_URL}/api/v1/conversations/${CONVERSATION_ID}/events"
```

预期结果：

- 初始出现 `event: conversation`
- 后续可能出现 `event: task`
- 后续可能出现 `event: lifecycle`

观察到需要的信息后手动停止即可。

## 快速结果矩阵

当没有 live workspace session 时，最小通过集为：

- Test 1
- Test 2
- Test 3
- Test 4
- Test 5
- Test 6
- Test 7
- Test 8

当存在 live workspace session 时，完整通过集为：

- Test 1 到 Test 13
