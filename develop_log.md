# 远程发起任务能力设计草案

## 背景

当前 CodexMonitor 已经具备两套关键能力：

1. Linux/桌面端可运行 CodexMonitor 本体或独立 daemon。
2. 后端已经支持按 workspace 管理 Codex 会话，并通过现有接口完成：
   - 连接 workspace
   - 创建 thread
   - 向 thread 发送用户消息
   - 跟踪执行过程与结果

现在希望补一个新场景：

- 不打开 CodexMonitor UI
- 在任意地点、任意设备上发起任务
- 让部署在 Linux 上的 CodexMonitor 接收请求并执行

目标是最小化改动，不重做任务系统。

## 核心判断

这个需求本质上不是“新增一个任务执行引擎”，而是“给现有 CodexMonitor/daemon 增加一个无界面触发入口”。

也就是说，最小方案不应该引入：

- 新的队列系统
- 新的调度中心
- 新的执行模型
- 新的线程存储模型

而应该直接复用现有模型：

- workspace 是执行上下文
- thread 是任务承载体
- `start_thread` / `send_user_message` 是现成执行入口

因此最小改动方向应为：

1. 在 daemon 侧增加一个聚合型 RPC。
2. 外部请求进入后，直接转成现有 thread/message 流程。
3. 返回 `threadId`，后续仍通过已有 UI 或 RPC 查看结果。

## 推荐方案

### 一句话方案

新增 daemon RPC：`task_submit`

该接口只是一个薄封装：

1. 校验请求参数
2. 定位 workspace
3. 如果未指定 `threadId`，创建 thread
4. 调用现有发送消息逻辑
5. 返回 `workspaceId`、`threadId`、受理结果

## 为什么这是最小改动

仓库当前架构已经具备以下基础：

- Tauri app 本地命令面
- daemon 远程 JSON-RPC 面
- shared core 共享业务逻辑
- workspace/session 生命周期管理
- thread 创建、恢复、发送消息能力
- 事件流与结果展示链路

因此，最优做法不是再加一套并行体系，而是在 daemon 上补一个“组合现有能力”的入口。

这符合仓库现有架构规则：

- 共享逻辑优先放 `src-tauri/src/shared/*`
- app 和 daemon 保持薄适配
- 不随意改变现有 payload/协议形状

## 建议能力边界

第一阶段只做“远程提交任务到指定 workspace”。

先不做：

- 自动选择 workspace
- 任务优先级
- 任务编排
- webhook 回调
- 独立任务数据库
- 新的任务列表 UI

原因是这些都会把“触发入口”问题扩大成“任务平台”问题，不符合最小改动目标。

## 第一阶段用户流程

### 流程 A：发起新任务

1. 外部客户端提交请求
2. daemon 收到 `task_submit`
3. 根据 `workspaceId` 找到目标 workspace
4. 若该 workspace 未连接，则走现有连接流程
5. 创建新 thread
6. 发送任务文本
7. 返回 `threadId`

### 流程 B：在现有 thread 继续追问

1. 外部客户端提交请求，附带 `threadId`
2. daemon 校验 thread 与 workspace 对应关系
3. 直接向该 thread 发送消息
4. 返回受理结果

## 接口草案

### RPC 名称

`task_submit`

### 请求参数

```json
{
  "workspaceId": "workspace_123",
  "text": "修复登录重定向 bug，并提交代码",
  "threadId": null,
  "model": null,
  "effort": null,
  "serviceTier": null,
  "accessMode": "full-access",
  "images": null,
  "collaborationMode": null,
  "appMentions": null
}
```

### 返回参数

```json
{
  "accepted": true,
  "workspaceId": "workspace_123",
  "threadId": "thread_abc",
  "createdThread": true
}
```

### 字段说明

- `workspaceId`：必须显式传入，避免第一阶段引入自动路由复杂度。
- `text`：任务主内容。
- `threadId`：可选；不传则新建 thread。
- `model` / `effort` / `serviceTier` / `accessMode`：沿用现有消息发送参数。
- `images` / `collaborationMode` / `appMentions`：建议先与现有消息发送能力对齐，是否在第一阶段开放可按实现复杂度裁剪。

## 后端实现思路

### 方案重点

`task_submit` 不应自己实现新的执行链路，只应编排现有能力。

伪流程：

```text
task_submit(request)
  -> validate workspaceId/text
  -> ensure workspace connected
  -> threadId =
       if request.threadId exists:
         use existing thread
       else:
         start_thread(workspaceId)
  -> send_user_message(workspaceId, threadId, text, options...)
  -> return accepted result
```

### 建议代码落点

优先级建议如下：

1. daemon RPC 路由：
   - `src-tauri/src/bin/codex_monitor_daemon/rpc.rs`
   - 或 `src-tauri/src/bin/codex_monitor_daemon/rpc/*`

2. 如果组合逻辑开始变复杂，下沉到 shared：
   - `src-tauri/src/shared/codex_core.rs`
   - 或新增一个与线程发送聚合相关的 shared helper

3. 如果未来本地 app 也需要复用同一能力，再补 Tauri command 面：
   - `src-tauri/src/lib.rs`
   - `src/services/tauri.ts`

第一阶段如果只是 Linux daemon 远程提交，理论上可以只改 daemon RPC 层和必要 shared 层，不必改前端。

## 外部接入方式对比

### 方案 1：直接调用 daemon JSON-RPC

优点：

- 改动最少
- 不增加新服务

缺点：

- 外部调用方需要理解当前 RPC 协议
- 不适合作为长期对外接口

适用：

- 自己写 CLI
- 自己写自动化脚本

### 方案 2：额外加一个极薄 HTTP bridge

优点：

- 对外接口简单
- daemon 内部协议可保持稳定
- 更方便接手机快捷指令、Webhook、Bot

缺点：

- 多一个小进程或小服务

适用：

- 追求长期可扩展
- 后续想接 Telegram/飞书/企业微信/自建网页表单

### 当前建议

先做 `task_submit` RPC，外部入口先不内建到 CodexMonitor 主仓库也可以。

也就是说，先定义稳定的“内部标准提交接口”，之后无论：

- CLI
- HTTP bridge
- Bot
- Shortcut

都只是在外面包一层。

## 安全边界

这部分必须前置考虑，因为该能力本质上允许远程触发开发任务执行。

第一阶段建议：

1. 不直接暴露 daemon 到公网。
2. 仅通过内网、Tailscale 或反向代理保护后访问。
3. 保留现有 token 认证。
4. `workspaceId` 必须显式传入，不做模糊匹配。
5. 建议默认限制允许远程触发的 workspace 白名单。
6. 默认不要自动给最激进的权限模式，除非调用方显式声明。

## 与现有架构的一致性

该方案与仓库规则一致：

- 不复制 app/daemon 逻辑
- 复用 shared core
- 保持 thread/workspace 模型不变
- 不引入新的 UI 状态系统
- 不破坏现有前后端契约

从维护角度看，它只是“现有后端能力的聚合入口”，不是新子系统。

## 可能的后续演进

如果第一阶段跑通，后续可以逐步扩展，但不应在首版一起做：

1. `task_submit_by_workspace_path`
   - 允许按路径查找 workspace

2. `task_submit_by_alias`
   - 给 workspace 增加远程调用别名

3. `task_status`
   - 返回 thread 当前状态摘要

4. `task_tail`
   - 返回最近消息或最近 agent 输出

5. 简单 Web 表单 / 移动端快捷入口
   - 只负责投递，不承担完整 UI

## 不建议的方向

以下方案在当前阶段都不建议优先做：

- 在 daemon 内直接实现一整套新的 HTTP 任务系统
- 引入数据库保存独立任务表
- 新做调度器/优先级/队列消费器
- 自动推断 workspace
- 同时支持复杂多任务编排

这些都不属于“最小化改动”。

## 推荐实施顺序

### 第一步

在 daemon 增加 `task_submit` RPC，先支持：

- `workspaceId`
- `text`
- `threadId?`
- `model?`
- `effort?`
- `accessMode?`

### 第二步

写一个最小调用端进行验证：

- CLI 脚本
- curl/http bridge
- 自己的自动化调用脚本

### 第三步

视使用情况决定是否补：

- 线程状态查询
- 结果摘要接口
- 更友好的对外 HTTP 包装

## 结论

这个需求的最小正确实现方式是：

- 不增加新的任务执行体系
- 不改变 workspace/thread 模型
- 在 daemon 侧新增一个聚合 RPC：`task_submit`
- 让外部请求复用现有 `start_thread` + `send_user_message` 逻辑

这样改动面最小，架构最稳定，也最符合当前仓库的设计原则。

## 中心化 Server-Client 模式扩展

上面的方案解决的是“远程发起任务”问题，但如果目标进一步明确为：

- 服务端统一部署 `codex + git + 项目代码 + CodexMonitor daemon`
- 任意电脑只作为请求入口和查看终端
- 不在客户端本地执行任何任务
- 所有会话、线程、执行上下文全部沉淀在服务端

那么更合适的产品模型应当定义为中心化的 server-client 模式。

## 模式定义

### 服务端职责

服务端是唯一执行中心，负责：

- 托管全部 workspace
- 托管全部 Codex 会话
- 托管全部 thread 历史
- 执行 Git、文件修改、review、worktree 等操作
- 统一保存任务结果与执行上下文

### 客户端职责

客户端只负责：

- 连接服务端
- 列出项目与线程
- 发消息
- 查看输出
- 处理审批和用户输入

客户端不负责：

- 本地拉起 codex
- 本地扫描项目
- 本地维护 thread 真正执行状态

这意味着客户端更像“远程聊天终端”，而不是本地 agent 宿主。

## 产品抽象建议

### 推荐抽象

建议采用以下三层抽象：

1. `workspace` = 项目
2. `thread` = 项目内一次独立任务/话题
3. `message` = 用户或 agent 在 thread 中的对话消息

从用户视角上，可以把它包装成：

- “每个项目像一个长期在线的开发助手”

但从系统实现上，不建议做成：

- “每个项目只有一条会话”

更合理的模型是：

- 一个项目对应一个长期助手入口
- 该项目下允许存在多条 thread
- 每条 thread 承载一类具体任务

例如：

- `project-a`
  - `修复登录重定向 bug`
  - `支付模块 review`
  - `PR #128 分析`
- `project-b`
  - `CI 失败排查`
  - `重构缓存层`

这样既保留了“像聊天”的自然体验，也保留了任务隔离与上下文清晰性。

## 为什么不建议“一个项目只有一个 thread”

如果把一个项目硬编码为单 thread，会带来明显问题：

1. 历史上下文会越来越混乱。
2. 不同任务相互污染，难以回溯。
3. review、实现、追问、实验都挤在一条会话里。
4. 后续要做并发会很难扩展。

因此，建议这样理解：

- 产品层：一个项目是一个助手
- 数据层：一个项目下有很多 thread

## 推荐交互模型

### 模型 A：显式线程模式

用户先进入项目，再选择 thread：

1. 打开项目
2. 查看 thread 列表
3. 进入某个 thread 发消息
4. 或创建新 thread

这是最贴近当前实现的模式，最容易落地。

### 模型 B：项目聊天入口模式

用户只对项目发消息：

1. 选择项目
2. 直接输入任务
3. 服务端决定：
   - 继续最近活跃 thread
   - 或新建一个 thread
4. 返回实际承载该任务的 `threadId`

这更像“好友聊天”，但底层依然应落在 thread 上。

### 当前建议

首版先保留显式 thread 模型作为底层事实；
后续若要优化产品体验，再在接口或 UI 上包装“项目助手入口”。

## 中心化模式下的最小接口集合

如果想支持任意电脑作为轻客户端接入，服务端至少需要以下能力：

### 基础接口

- `workspace_list`
- `thread_list(workspaceId)`
- `thread_read(workspaceId, threadId)`
- `task_submit(workspaceId, text, threadId?)`

这四个接口就足以支撑一个简化版“聊天客户端”。

### 扩展接口

后续可以增加：

- `workspace_summary(workspaceId)`
- `thread_status(workspaceId, threadId)`
- `thread_recent_messages(workspaceId, threadId, limit)`
- `workspace_recent_activity`

### 审批与交互接口

如果客户端需要完整参与执行过程，还需要暴露：

- `approval_list`
- `approval_respond`
- `user_input_list`
- `user_input_respond`

不过从现有代码结构看，很多这类能力已经存在，只是目前主要服务前端 UI。

## 与当前项目现状的衔接

这个中心化模式与当前架构不是冲突关系，而是顺势加强。

当前项目已经具备：

- daemon 远程运行能力
- workspace 统一存储
- thread 统一恢复与读取
- 事件驱动更新
- 远程 backend 模式

所以它不是从零开始做 server-client，而是把已有“远程模式”进一步明确为“中心化主模式”。

## 建议演进路线

### 阶段 1：最小可用远程提交

先完成本文前半部分定义的：

- `task_submit`

目标：

- 任意地方可以向某个项目发起任务
- thread 与结果都沉淀在服务端

### 阶段 2：最小远程会话读取

补齐只读查询接口：

- `workspace_list`
- `thread_list`
- `thread_read`

目标：

- 任意客户端可以查看服务端已有项目与对话

### 阶段 3：轻客户端落地

提供一个最小 client 形态，可能是：

- 轻网页
- 极薄桌面端
- 命令行聊天入口

目标：

- 客户端只作为壳，不承担执行责任

### 阶段 4：项目助手入口

在产品层加一层更友好的抽象：

- 进入一个项目直接发消息
- 系统自动选择或创建 thread

目标：

- 让系统更像“和项目助手聊天”

## 并发策略建议

中心化模式下，一个新问题会更突出：同一项目能不能同时执行多个任务。

### 保守建议

首版默认不要允许同一 workspace 中无限制并发写任务。

原因：

- 同一代码树中的写操作容易冲突
- 多个 agent 同时修改同一仓库风险很高

### 推荐策略

1. 同一主 workspace 默认单活跃写任务。
2. 如果需要并发任务，优先通过 `worktree` 或 `clone` 派生隔离环境。
3. 读操作、review 类任务可以适当放宽。

这与项目当前已有的 worktree/clone 设计天然兼容。

## “一个项目一个 agent 助手”的实现建议

这个方向是合理的，但建议分成两层来实现。

### 产品语义层

对用户展示为：

- 一个项目对应一个长期助手入口

用户可以理解为自己“在和这个项目的助手聊天”。

### 系统实现层

系统内部依然保留：

- 一个项目下多个 thread
- 一个 thread 对应一个任务上下文
- 并发时通过 worktree/clone 做隔离

这样既保留长期记忆感，也不会牺牲线程隔离能力。

## 会话集中管理的优势

如果采用中心化 server-client 模式，CodexMonitor 的价值会更清晰：

1. 所有对话沉淀在服务端，不随设备漂移。
2. 所有项目环境统一，不受客户端环境影响。
3. 权限、依赖、Codex 版本、Git 凭证都集中维护。
4. 一个项目的长期上下文更容易积累。
5. 手机、平板、临时电脑都能成为入口。

## 需要补的治理能力

如果这个模式未来不只是个人使用，而是多人接入，还要补以下能力：

### 认证与授权

- token 分级
- workspace 访问白名单
- 只读/可执行权限区分

### 请求来源记录

建议为任务记录：

- 来源客户端
- 提交时间
- 可选用户标识

这样后续 thread 历史会更清晰。

### 通知能力

后续可以考虑：

- 任务完成通知
- 审批待处理通知
- 失败通知

但不建议在第一阶段引入。

## 对当前文档主方案的修正

如果最终目标是中心化 server-client，而不只是“偶发远程提交”，那么 `task_submit` 只是第一步，不是终点。

完整方向应理解为：

1. 先补远程提交能力
2. 再补远程读取能力
3. 再把客户端彻底轻量化
4. 最后把产品抽象提升为“项目助手”

## 扩展结论

从长期演进看，CodexMonitor 完全可以朝这个方向发展：

- 服务端统一部署 Codex 和项目
- 客户端只负责聊天与查看
- 一个项目对应一个长期助手入口
- 底层仍以多 thread 模型承载实际任务

因此，“一个项目开启一个 agent 助手”是合适的产品表达；
但实现上应当是：

- 一个项目 = 一个助手空间
- 一个项目下 = 多条任务 thread
- 并发执行 = 通过 worktree/clone 隔离

这是兼顾最小改动、产品体验和后续可扩展性的更稳妥方案。
