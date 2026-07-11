# CodexMonitor 服务化闭环开发草案 V1

## 目标

先把整条链路跑起来，先做单链路闭环，不做高复杂多人并发，不做多租户调度平台，不做复杂授权池负载均衡。

V1 目标只有一条：

1. 业务侧编辑需求
2. 点击开发
3. 请求发到服务器上的 CodexMonitor daemon
4. daemon 接收会话请求并定位执行上下文
5. 创建或复用对话 thread
6. 实时返回对话生命周期和输出内容
7. 可以查看对话结果和历史记录

一句话定义：

`把 CodexMonitor daemon 先做成一个单链路、可观测、可追溯的需求开发中台。`

更具体地说，V1 要做成：

`一个面向 Web 前端的 HTTP 对话服务。`

也就是：

- Web 页面通过 HTTP 发起对话
- Web 页面通过 HTTP 查询历史对话
- Web 页面通过 SSE 订阅实时生命周期和增量输出
- daemon 内部再去复用现有 thread / task / workspace 能力

## 当前判断

这个项目已经不是从零开始。

仓库里已经有：

- daemon 进程
- TCP JSON-RPC
- HTTP bridge
- task 资源雏形
- SSE 事件流雏形
- workspace / session / thread / message 现成能力

所以 V1 不应该另起一套后端，不应该新造任务执行引擎，也不应该切到另一套服务框架。

最小可行路线：

- 继续以 `codex_monitor_daemon` 为唯一服务宿主
- 扩 HTTP API，让它成为 Web 对话主入口
- 复用现有 `start_thread`、`resume_thread`、`send_user_message`
- 增加一层面向业务的 `conversation` 抽象
- TCP JSON-RPC 继续保留，但它是内部能力，不是 Web 接入主协议

## V1 范围

### 要做

- 面向 Web 的 HTTP 对话入口
- 发起开发对话
- 继续追问
- 查询对话详情
- 查询历史对话列表
- SSE 实时查看生命周期和增量输出
- 保存 conversation 元数据
- 查看最终结果

### 不做

- 高复杂多人并发
- 授权池动态调度
- 多产品抢占资源调度
- 复杂数据库建模
- WebSocket
- 消息级双向强同步
- 分布式部署
- 复杂权限系统

## V1 业务模型

V1 先固定 5 个对象。

### 1. workspace

实际执行目录，对应当前系统已有 workspace 模型。

V1 不重做，直接复用 daemon 当前已有的 workspace 注册和连接能力。

### 2. conversation

这是业务侧主对象。

用户点击“开发”后，业务系统最关心的是 `conversationId`，而不是底层 `threadId`。

建议字段：

- `conversationId`
- `workspaceId`
- `threadId`
- `title`
- `requirement`
- `status`
- `operator`
- `createdAt`
- `updatedAt`
- `lastMessagePreview`
- `finalSummary`
- `lastError`

### 3. thread

这是 Codex 真实执行对话。

V1 仍然保留 thread 作为底层事实载体，但对业务侧尽量不直接暴露 thread 模型细节。

### 4. task

一次具体提交动作。

例如：

- 第一次点击“开发”
- 对已有对话继续补一句话

V1 可以继续复用 daemon 已有 `task` 结构，不需要重造。

## 对象关系

- 一个 `conversation` 对应一个 `thread`
- 一个 `conversation` 下可以有多次 `task`
- Web 前端主键优先使用 `conversationId`
- 服务端内部映射到 `workspaceId + threadId`
- 执行上下文可通过请求参数或外部文件配置传入，不作为 V1 核心对象

## V1 核心流程

### 流程 A：点击开发，发起新需求

1. Web 前端填写标题和需求
2. Web 前端调用 `POST /api/v1/conversations/start`
3. daemon 从请求参数或文件配置中拿到执行上下文
4. daemon 校验 workspace 是否存在
5. 若 workspace 未连接，则走现有连接流程
6. daemon 创建 thread
7. daemon 拼装真正发送给 Codex 的 prompt
8. daemon 调用现有 `send_user_message`
9. daemon 创建 `conversation` 元数据
10. 返回：
   - `conversationId`
   - `workspaceId`
   - `threadId`
   - `taskId`
   - `status`
11. Web 前端开始订阅 SSE
12. Web 前端实时显示生命周期和增量输出
13. 对话完成后可查看结果

### 流程 B：继续追问

1. Web 前端打开已有 `conversation`
2. 输入补充要求
3. 调用 `POST /api/v1/conversations/{conversationId}/messages`
4. daemon 查到对应 `threadId`
5. daemon 继续向该 thread 发送消息
6. 返回新的 `taskId`
7. Web 前端继续订阅相同 conversation 的事件流

### 流程 C：查看结果

1. Web 前端请求 `GET /api/v1/conversations/{conversationId}`
2. daemon 返回 conversation 摘要
3. Web 前端请求 `GET /api/v1/conversations/{conversationId}/messages`
4. daemon 从 thread 读取完整消息
5. Web 前端展示最终结果与完整上下文

## 生命周期设计

V1 不要暴露太细的底层状态，先统一成 Web 前端可直接消费的状态。

建议状态：

- `accepted`
- `thread_created`
- `running`
- `streaming`
- `waiting_input`
- `waiting_approval`
- `completed`
- `failed`

### 状态说明

- `accepted`
  服务端已接收请求
- `thread_created`
  thread 已创建成功
- `running`
  Codex 已开始处理
- `streaming`
  正在持续输出
- `waiting_input`
  等待用户补充信息
- `waiting_approval`
  等待审批或确认
- `completed`
  对话本轮完成
- `failed`
  执行失败

### 内部映射原则

服务端内部仍可保留底层 `task.status`、`turn`、`AppServerEvent` 等信息，但对外先映射成上面这套有限状态。

不要让 Web 前端直接依赖底层 event 名称。

## 执行上下文输入

V1 不把“产品线配置能力”当核心建设项。

执行上下文先允许两种最小输入方式：

- 请求参数直接传入
- 服务端读取外部文件路径

也就是 V1 只要求 daemon 能拿到这些信息：

- `workspaceId`
- `codexProfile`
- `defaultPromptTemplate`

至于这些值来自：

- 前端参数
- 外部配置文件
- 启动参数

都可以，先不在当前阶段做复杂产品线配置中心。

## Prompt 组装建议

Web 前端输入的是业务需求，不应直接原样裸发。

V1 建议服务端统一组装：

```text
[产品线默认提示]

需求标题：
<title>

需求描述：
<requirement>

要求：
1. 先基于当前代码库理解现状
2. 输出实现过程和最终结果
3. 若需要补充信息，明确提出
```

这样不同来源的执行上下文也能保持最基本的一致性。

## 历史对话存储策略

V1 不要上来做复杂数据库。

先拆成两类数据：

### 1. conversation 元数据

这是服务端必须自己保存的。

建议保存：

- `conversationId`
- `workspaceId`
- `threadId`
- `title`
- `requirement`
- `status`
- `operator`
- `createdAt`
- `updatedAt`
- `lastMessagePreview`
- `finalSummary`
- `lastError`

建议先落本地 JSON 文件，或者复用 daemon data-dir 下新的存储文件。

例如：

- `conversations.json`

### 2. 完整消息内容

V1 不单独重复存储全文。

先继续依赖 thread 读取能力：

- `resume_thread`
- `read_thread`

也就是说：

- 元数据自己存
- 全量消息先从已有 thread 恢复

这样最省改动。

## 历史数据处理设计补充

当前更合理的方向不是让 Web 直接依赖 daemon 内存或 thread 实时恢复，而是：

`先由 codex-monitor 写历史，再由 Web 侧服务读历史。`

建议职责拆分：

- `codex`
  负责任务执行、thread 对话、底层事件产出
- `codex-monitor`
  负责接 HTTP 对话请求、监听对话生命周期、写 MySQL、提供实时 SSE
- `product-fastapi`
  负责查 MySQL，把历史会话、消息、结果提供给 Web

这样拆的核心是：

- `codex-monitor` 是写模型
- `product-fastapi` 是读模型

V1 最简单的链路：

1. Web 发起需求到 `codex-monitor`
2. `codex-monitor` 调用 `codex`
3. `codex-monitor` 监听 thread / task / turn 变化
4. `codex-monitor` 把消息和生命周期写入 MySQL
5. Web 实时查看走 SSE
6. Web 历史查看走 `product-fastapi -> MySQL`

## 服务职责补充

### codex

负责：

- 创建 thread
- 执行对话
- 输出 assistant 内容
- 输出底层 task / turn / thread 事件

不负责：

- 历史落库
- 面向 Web 的历史查询
- 生命周期归档

### codex-monitor

负责：

- 面向 Web 的 HTTP 对话入口
- 调用现有 codex thread 能力
- 管理 `conversationId -> threadId` 映射
- 监听生命周期
- 写 MySQL
- 提供实时 SSE

这是 V1 的核心服务。

### product-fastapi

负责：

- 历史对话查询
- 消息列表查询
- 最终结果查询
- 生命周期记录查询
- 面向 Web 提供读接口

不负责：

- 驱动 codex 执行
- 写入对话消息
- 处理实时 thread 生命周期

### 当前实现落点

`product-fastapi` 当前不单独新建项目，直接使用现有：

- `/root/project/RuoYi-Vue3-FastAPI/ruoyi-fastapi-backend`

作为后台开发承载。

原因：

- 该项目本身就是 FastAPI 后端
- 已接 MySQL
- 已有清晰的 `controller / service / dao / entity` 分层
- 更适合直接增加“对话历史读取”模块

因此当前建议不是：

- 新起一个独立 FastAPI 服务

而是：

- 在 `RuoYi-Vue3-FastAPI` 后端内新增会话历史查询模块

## RuoYi-FastAPI 承接建议

当前 `/root/project/RuoYi-Vue3-FastAPI/ruoyi-fastapi-backend` 已具备模块化结构：

- `module_xxx/controller`
- `module_xxx/service`
- `module_xxx/dao`
- `module_xxx/entity`

所以最小可行方式是新增一个对话模块，例如：

- `module_codex/controller`
- `module_codex/service`
- `module_codex/dao`
- `module_codex/entity`

这个模块只做读，不做写。

负责：

- 查询 conversation 列表
- 查询 conversation 详情
- 查询 conversation 消息
- 查询 conversation 生命周期事件

不负责：

- 发起 codex 对话
- 驱动 thread
- 实时 SSE
- 写入 MySQL

## 实时与历史分流

V1 建议明确分成两条链路。

### 1. 实时链路

- Web -> `codex-monitor`
- `codex-monitor` -> SSE -> Web

这条链路负责：

- 当前状态
- 增量输出
- 生命周期变化

### 2. 历史链路

- Web -> `product-fastapi`
- `product-fastapi` -> MySQL

这条链路负责：

- 历史会话列表
- 历史消息详情
- 最终结果
- 生命周期记录

这样最简单，也最稳定。

这里的 `product-fastapi` 默认就是：

- `RuoYi-Vue3-FastAPI/ruoyi-fastapi-backend`

## MySQL 存储建议

V1 可以允许 daemon 本地文件继续存在，但如果要支撑 Web 稳定查历史，建议补 MySQL 作为历史事实库。

最少建议 4 张表。

### 1. conversation

存会话主记录。

建议字段：

- `id`
- `workspace_id`
- `thread_id`
- `title`
- `requirement`
- `status`
- `operator`
- `final_summary`
- `last_error`
- `created_at`
- `updated_at`

### 2. conversation_message

存对话消息。

建议字段：

- `id`
- `conversation_id`
- `thread_id`
- `turn_id`
- `role`
- `message_type`
- `content`
- `sequence_no`
- `created_at`

### 3. conversation_event

存生命周期事件。

建议字段：

- `id`
- `conversation_id`
- `thread_id`
- `turn_id`
- `event_type`
- `event_status`
- `payload_json`
- `created_at`

### 4. conversation_task

存任务提交与执行状态。

建议字段：

- `id`
- `conversation_id`
- `task_id`
- `status`
- `created_thread` (`TINYINT(1)` / boolean 标记；1=本次任务新建 thread，0=复用已有 thread)
- `submitted_at`
- `completed_at`
- `last_error`

## V1 写库原则

V1 不要过度设计，按最小闭环写。

建议：

- conversation 创建时：写 `conversation`
- 用户消息发送时：写 user message
- assistant 完成输出时：写最终 assistant message
- 生命周期变化时：写 `conversation_event`
- task 状态变化时：写 `conversation_task`
- 完成时：更新 `conversation.status`、`final_summary`

## V1 消息落库策略

第一版不建议把每个增量 token 都落库。

建议：

- 实时增量只走 SSE
- assistant 完成后再合并写一条完整消息到 MySQL

这样可以明显降低复杂度。

## 查询职责补充

### codex-monitor 提供

- 发起会话
- 继续追问
- SSE 实时事件

### product-fastapi 提供

- `GET /conversations`
- `GET /conversations/{id}`
- `GET /conversations/{id}/messages`
- `GET /conversations/{id}/events`

这样 Web 历史页就不需要直接读 daemon。

当前这些接口建议直接落在 `RuoYi-Vue3-FastAPI` 后端模块里。

## 当前推荐结论

历史数据处理建议定成：

- `codex` 负责任务处理
- `codex-monitor` 负责消息写入 MySQL + 生命周期归档 + 实时 SSE
- `product-fastapi` 负责历史读取

这是当前最简单、最稳、最容易先跑起来的结构。

## 最终结果展示

业务侧通常不想读满屏原始对话，更想直接看结论。

V1 建议在 `conversation` 上保存：

- `finalSummary`
- `lastMessagePreview`

更新规则：

- 对话完成时，取最后一条 assistant 输出
- `lastMessagePreview` 存简短预览
- `finalSummary` 可先直接截取最后一条文本前 500 到 1000 字

先别做复杂智能总结。

## 对外 HTTP API 草案

这是 V1 的核心主接口面。

外部接入方默认是 Web 前端，不是 TCP 客户端，不是桌面内部 RPC 调用。

V1 先做面向业务的接口，而不是把 thread API 原样暴露出去。

### 1. 发起开发

`POST /api/v1/conversations/start`

请求：

```json
{
  "workspaceId": "ws-member",
  "codexProfile": "codex-team-member",
  "defaultPromptTemplate": "你负责会员产品线，请按业务需求给出直接实现。",
  "title": "新增会员积分过期提醒",
  "requirement": "请基于当前项目实现会员积分过期前3天提醒功能",
  "operator": "zhangsan"
}
```

响应：

```json
{
  "conversation": {
    "conversationId": "conv_001",
    "workspaceId": "ws-member",
    "threadId": "thread_abc",
    "status": "accepted",
    "title": "新增会员积分过期提醒"
  },
  "task": {
    "taskId": "task_001",
    "status": "accepted"
  }
}
```

### 2. 继续追问

`POST /api/v1/conversations/{conversationId}/messages`

请求：

```json
{
  "text": "请直接给出最小改动方案，并说明涉及哪些文件",
  "operator": "zhangsan"
}
```

响应：

```json
{
  "conversationId": "conv_001",
  "task": {
    "taskId": "task_002",
    "status": "accepted"
  }
}
```

### 3. 查看对话详情

`GET /api/v1/conversations/{conversationId}`

响应：

```json
{
  "conversation": {
    "conversationId": "conv_001",
    "workspaceId": "ws-member",
    "threadId": "thread_abc",
    "title": "新增会员积分过期提醒",
    "status": "completed",
    "lastMessagePreview": "已完成会员积分过期前3天提醒逻辑...",
    "finalSummary": "已完成该功能，涉及会员积分查询、到期计算和提醒发送。",
    "lastError": null
  }
}
```

### 4. 查看完整消息

`GET /api/v1/conversations/{conversationId}/messages`

说明：

- 服务端通过 `workspaceId + threadId` 读取现有 thread 消息
- 返回格式可贴近当前 thread message 结构

### 5. 查看历史对话列表

`GET /api/v1/conversations`

响应：

```json
{
  "items": [
    {
      "conversationId": "conv_001",
      "title": "新增会员积分过期提醒",
      "status": "completed",
      "updatedAt": 1760000000000,
      "lastMessagePreview": "已完成会员积分过期前3天提醒逻辑..."
    }
  ]
}
```

### 6. 订阅对话事件

`GET /api/v1/conversations/{conversationId}/events`

传输方式：

- SSE

说明：

- 这是 Web 实时对话的主通道
- 不要求 Web 前端直接接 TCP JSON-RPC
- 浏览器只需要 HTTP + SSE 就能跑完整闭环

V1 事件类型建议：

- `conversation.accepted`
- `conversation.thread_created`
- `conversation.running`
- `conversation.streaming`
- `conversation.waiting_input`
- `conversation.waiting_approval`
- `conversation.completed`
- `conversation.failed`
- `conversation.message.delta`

示例：

```text
event: conversation.running
data: {"conversationId":"conv_001","status":"running","timestamp":1760000000000}

event: conversation.message.delta
data: {"conversationId":"conv_001","text":"正在分析当前代码结构...","timestamp":1760000000100}
```

## 前端页面最小闭环

V1 Web 前端只需要 3 块。

### 1. 需求发起区

- 填执行上下文或选择外部配置
- 输入标题
- 输入需求
- 点击开发

### 2. 实时对话区

- 展示当前状态
- 展示增量输出
- 展示生命周期事件

### 3. 历史对话区

- 展示最近 conversations
- 点击进入详情
- 支持继续追问

## 服务端改造建议

### 第一层：配置层

V1 只保留最小配置输入能力。

负责拿到：

- `workspaceId`
- `codexProfile`
- `defaultPromptTemplate`

来源可以是请求参数或外部文件路径。

### 第二层：conversation 元数据层

新增 conversation 存储。

例如：

- `conversations.json`

负责维护：

- conversation 基本信息
- 状态
- threadId 映射
- 最终结果摘要

### 第三层：服务层

新增业务型服务接口：

- `start_conversation`
- `append_conversation_message`
- `get_conversation`
- `list_conversations`
- `read_conversation_messages`
- `subscribe_conversation_events`

要求：

- 内部调用现有 workspace / thread / task 能力
- 不复制底层业务逻辑

### 第四层：HTTP 层

在 daemon 当前 HTTP API 上增加 conversation 相关路由。

V1 明确要求：

- Web 只走 HTTP
- 实时只走 SSE
- TCP JSON-RPC 只作为 daemon 内部或桌面适配层能力保留

## 后端当前缺少的待开发能力

下面只列 V1 真正缺的后端能力，目的是补齐“产品需求发起 -> 实时跟踪 -> 查看结果”的闭环。

### 1. 执行上下文输入能力

当前 daemon 已有 workspace、thread、task 基础能力，但“产品线配置中心”不是当前核心。

后端缺少：

- 执行上下文参数接收
- 外部文件路径读取
- `workspaceId`、`codexProfile`、`defaultPromptTemplate` 基础校验

没有这层，Web 前端或上层系统无法把对话发到正确执行上下文。

### 2. conversation 元数据存储能力

当前已有 thread 和 task，但还没有业务侧可直接使用的 `conversation` 记录层。

后端缺少：

- `conversationId` 生成
- `conversation` 元数据持久化
- `conversationId -> workspaceId + threadId` 映射
- conversation 状态更新
- 历史 conversation 列表读取

没有这层，Web 前端只能拿底层 `threadId`，不适合作为产品化接口。

### 3. 面向业务的发起开发接口

当前已有 `/api/v1/tasks` 和底层 thread/task 能力，但还没有“按会话发起开发”的业务接口。

后端缺少：

- `POST /api/v1/conversations/start`
- 接收执行上下文参数
- 按产品模板组装 prompt
- 创建 conversation 并关联 thread/task

没有这层，Web 前端仍然要理解 workspace/thread 细节，接入成本高。

### 4. 已有 conversation 继续追问接口

当前可以直接往 thread 发消息，但没有 conversation 维度的补充提问入口。

后端缺少：

- `POST /api/v1/conversations/{conversationId}/messages`
- 根据 `conversationId` 查 thread
- 对 conversation 补写最近活跃时间和状态

没有这层，Web 前端无法用 conversation 主键继续对话。

### 5. conversation 查询接口

当前缺的是业务层查询，不是底层 thread 查询。

后端缺少：

- `GET /api/v1/conversations/{conversationId}`
- `GET /api/v1/conversations`
- conversation 摘要返回结构
- 按 product/status/time 的基础筛选

没有这层，Web 前端无法稳定展示“历史需求开发记录”。

### 6. conversation 消息读取接口

当前已有 thread 恢复/读取能力，但没有封装成 conversation 维度的 HTTP 接口。

后端缺少：

- `GET /api/v1/conversations/{conversationId}/messages`
- `conversationId -> threadId` 转换
- thread 消息到业务响应结构的轻量转换

没有这层，Web 前端查看完整对话仍需直接依赖底层 thread API。

### 7. conversation 生命周期事件流

当前 daemon 已有 task SSE、thread SSE 雏形，但没有面向业务的 conversation 事件流。

后端缺少：

- `GET /api/v1/conversations/{conversationId}/events`
- conversation 生命周期事件映射
- task/thread/app-server event 到 conversation event 的统一转换
- 前端可直接消费的状态枚举

没有这层，Web 前端无法稳定显示“已接收 / 处理中 / 输出中 / 完成 / 失败”。

### 8. conversation 最终结果摘要能力

当前可以读原始消息，但没有面向列表和详情页的结果摘要字段。

后端缺少：

- `lastMessagePreview` 更新
- `finalSummary` 写入
- conversation 完成时的结果归档

没有这层，历史列表只能展示原始 thread，不适合快速查看“这次需求最后做成了什么”。

### 9. conversation 与 task 的关联能力

当前 task 是独立资源，conversation 还没接上。

后端缺少：

- `conversationId -> taskId[]` 关联
- 最新 task 状态回写 conversation
- task 失败原因同步到 conversation

没有这层，Web 前端无法从 conversation 直接看到本轮开发是否失败、失败在哪。

### 10. 最小可用的数据文件

V1 不上复杂数据库，但至少要补两个最小存储入口。

后端缺少：

- `conversations.json`

如果执行上下文走文件方式，再补一个外部配置文件即可；它不是 V1 核心存储。

## V1 后端最小开发清单

如果只按“先跑起来”做，后端最少补这 8 项：

1. 新增 `conversations.json`
2. 新增 `POST /api/v1/conversations/start`
3. 新增 `POST /api/v1/conversations/{conversationId}/messages`
4. 新增 `GET /api/v1/conversations/{conversationId}`
5. 新增 `GET /api/v1/conversations`
6. 新增 `GET /api/v1/conversations/{conversationId}/messages`
7. 新增 `GET /api/v1/conversations/{conversationId}/events`
8. 补执行上下文参数或文件路径读取

做到这里，后端就具备最小闭环能力。

## 推荐落地顺序

### Phase 1

打通最小闭环。

1. 新增 `conversations.json`
2. 新增 `POST /api/v1/conversations/start`
3. 新增 `GET /api/v1/conversations/{conversationId}`
4. 新增 `GET /api/v1/conversations/{conversationId}/events`
5. 补执行上下文参数或文件路径读取
6. Web 前端可以发起需求并看到实时输出

### Phase 2

补历史和继续追问。

1. 新增 `POST /api/v1/conversations/{conversationId}/messages`
2. 新增 `GET /api/v1/conversations`
3. 新增 `GET /api/v1/conversations/{conversationId}/messages`
4. Web 前端可查看历史记录和继续追问

### Phase 3

补运营性细节。

1. conversation 结果摘要
2. 失败原因展示
3. 基础审计字段

## 风险与约束

### 1. 先不解决高并发

当前设计明确只服务于：

- 单链路可用
- 少量请求
- 先验证业务闭环

不要在 V1 为未来并发提前上复杂架构。

### 2. 一个 conversation 只绑定一个 thread

这会让业务语义最清晰，V1 不要搞 conversation 下多 thread 分支。

### 3. 完整消息先不双写

先只存元数据，消息全文继续从 thread 恢复，避免引入第二份事实源。

### 4. 执行上下文先外部传入

workspace / codexProfile / promptTemplate 先由参数或文件给出，不做调度池。

## V1 完成标准

满足以下条件就算 V1 跑通：

1. Web 前端可提交需求到指定执行上下文
2. daemon 能创建 conversation 并驱动底层 thread
3. Web 前端能实时看到生命周期和输出
4. 对话完成后可查看结果
5. 可以打开历史对话并继续追问

## 最终判断

V1 的正确做法不是“做一个复杂任务平台”，而是：

`把现有 CodexMonitor daemon 从薄桥接层推进成一个可直接承载业务对话闭环的服务层。`

先把：

- 执行上下文输入
- conversation 元数据
- HTTP API
- SSE 生命周期
- 结果查看

这五件事做完，系统就已经能用。

后面再谈并发、调度、权限、扩展。
