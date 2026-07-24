# ragent Stream And Model Design Notes - 2026-05-24

本文记录 `D:/Study/project/agent/ragent` 在 SSE、模型流、取消和路由降级上的可借鉴思想，并说明这些思想如何映射到 `rove`。

这不是实现计划，也不代表已经锁定 API 变更。后续如果进入实现，需要另写 spec 和 implementation plan。

## Scope

本次重点参考以下 ragent 模块：

| Area | ragent file | Main idea |
|---|---|---|
| Chat SSE endpoint | `bootstrap/src/main/java/com/nageoffer/ai/ragent/rag/controller/RAGChatController.java` | 对话流入口和 stop endpoint 分离 |
| SSE protocol | `bootstrap/src/main/java/com/nageoffer/ai/ragent/rag/enums/SSEEventType.java` | 稳定事件名：`meta`、`message`、`finish`、`done`、`cancel`、`reject` |
| SSE sender | `framework/src/main/java/com/nageoffer/ai/ragent/framework/web/SseEmitterSender.java` | sender 生命周期封装，发送、完成、失败走同一边界 |
| Stream task manager | `bootstrap/src/main/java/com/nageoffer/ai/ragent/rag/service/handler/StreamTaskManager.java` | `taskId` 绑定 sender、取消句柄和取消时 payload |
| Chat stream handler | `bootstrap/src/main/java/com/nageoffer/ai/ragent/rag/service/handler/StreamChatEventHandler.java` | 模型回调转 SSE，聚合内容，完成或取消时收口 |
| Frontend stream consumer | `frontend/src/hooks/useStreamResponse.ts` | 前端按事件类型消费流，支持取消和 retry |
| Model client interface | `infra-ai/src/main/java/com/nageoffer/ai/ragent/infra/chat/ChatClient.java` | provider-agnostic sync/stream model boundary |
| Stream callback | `infra-ai/src/main/java/com/nageoffer/ai/ragent/infra/chat/StreamCallback.java` | `content`、`thinking`、`complete`、`error` 分离 |
| Cancellation handle | `infra-ai/src/main/java/com/nageoffer/ai/ragent/infra/chat/StreamCancellationHandle.java` | 模型流显式取消句柄 |
| Stream fallback barrier | `infra-ai/src/main/java/com/nageoffer/ai/ragent/infra/chat/ProbeStreamBridge.java` | 首包探测成功前不向下游提交内容 |
| Routing LLM service | `infra-ai/src/main/java/com/nageoffer/ai/ragent/infra/chat/RoutingLLMService.java` | 候选模型选择、健康状态、首包失败 fallback |

## What ragent Gets Right

### 1. Stream protocol is explicit

ragent 的 SSE 协议把生命周期拆得很清楚：

- `meta`: 一开始返回 `conversationId` 和 `taskId`。
- `message`: 模型增量内容，内部再区分 `response` 和 `think`。
- `finish`: 模型输出完成，携带持久化后的 `messageId`、标题等业务元信息。
- `done`: 流真正结束。
- `cancel`: 用户取消，携带取消时可保存的 completion payload。
- `reject`: 排队、限流或业务拒绝。

这个设计的价值不在事件名本身，而在于前后端共享一套明确的状态机。前端不需要从连接关闭、HTTP error 或最后一个 message 里反推流是否结束。

### 2. Cancellation has a real binding point

`StreamTaskManager` 的核心思想是把一个 streaming task 拆成三部分：

- `sender`: 往 SSE 发事件。
- `handle`: 模型流的取消句柄。
- `onCancelSupplier`: 取消时如何构造可落库或可返回给前端的 payload。

它还处理了一个重要竞态：用户可能在模型流真正创建之前就点取消。ragent 的做法是先记录 cancelled 状态；等 `bindHandle` 发生时，如果任务已取消，就立即调用 `handle.cancel()`。

这比“点取消时如果 handle 不存在就算了”可靠很多。

### 3. Terminal paths are centralized

正常完成、取消、拒绝、错误都必须走清晰的终态路径：

- 正常完成：保存 assistant message，发 `finish`，再发 `done`，unregister，complete emitter。
- 取消：保存部分内容，发 `cancel`，再发 `done`，complete emitter。
- 拒绝：仍然发一组可消费的终态事件，而不是只返回普通错误。

这个思想对 `rove` 很重要：agent run 比 chat response 长，终态如果不统一，Web UI、trace、report 和 resume 很容易出现不一致。

### 4. Model streaming is provider-agnostic

ragent 的模型层不是让业务代码直接解析某个 provider 的 HTTP stream，而是通过：

- `ChatClient`
- `LLMService`
- `StreamCallback`
- `StreamCancellationHandle`

把 provider 差异挡在 adapter 内部。OpenAI-compatible、Ollama、DeepSeek 等 provider 可以分别解析自己的格式，但上层只收到 content、thinking、complete、error。

这也是 `rove` 后续做 native tool-use 时应该走的方向。

### 5. First-packet fallback avoids leaking bad streams

`ProbeStreamBridge` 的作用是建立一个 commit barrier：

1. 先启动候选模型流。
2. 在首包成功、错误、超时、空响应之间做判断。
3. 首包成功前先缓冲回调，不交给下游。
4. 成功后再 commit，失败则取消当前 handle，尝试下一个候选模型。

这个设计避免了“主模型先吐了一点坏内容，随后又切 fallback”的用户体验和状态一致性问题。

## What rove Should Not Copy Directly

### 1. Do not replace the job API with chat-only SSE

ragent 的 `/rag/v3/chat` 是单次 RAG chat 场景；`rove` 是 workspace agent runtime，已经有：

- `POST /jobs`
- `GET /jobs/{id}/events`
- `GET /jobs/{id}/state`
- `POST /jobs/{id}/cancel`
- approval/input continuation endpoints

`rove` 应该保留 job/run/event-history 架构。ragent 的 `meta/message/finish/done` 思想可以借鉴，但不能把 API 形态改成纯 chat stream。

### 2. Redis distributed cancellation is not a current need

ragent 的 `StreamTaskManager` 用 Redis bucket/topic 做跨节点取消。这个对多实例部署有价值，但 `rove` 当前是 local-first runtime，不应该现在引入 Redis 级依赖。

可以先保留本进程内的 job registry、cancel token 和 stream handle。等将来有多节点 API 服务再考虑分布式 cancel bus。

### 3. Manual fetch SSE parser is optional

ragent 前端用 `fetch + ReadableStream` 手动解析 SSE，主要收益是：

- 可以带自定义 header。
- 可以直接使用 `AbortController`。
- retry 逻辑完全自控。

`rove` 当前的 `EventSource` 并非必须替换。更重要的问题是服务端 SSE 没有事件 `id`，前端 reducer 也不幂等。先修复 replay 和 dedupe，比换 parser 更有价值。

### 4. Queue limiter is a later concern

ragent 的公平分布式限流队列很完整，但现在对 `rove` 偏重。`rove` 需要先解决单 job stream 可靠性、模型事件结构化和工具取消。排队/并发限制可以后置到 API 多用户阶段。

## Mapping To rove

### SSE/job stream

当前 `rove` 的 API 已有历史事件 replay，但还缺少可靠 resume 的关键字段。建议吸收 ragent 的“稳定终态协议”思想，并按 rove 的 job 模型落地：

- 给每个 stored stream event 加单调递增 `seq`。
- SSE 输出 `id: <seq>`。
- `/jobs/{id}/events` 支持 `Last-Event-ID` 或 `?after=<seq>`。
- 后端避免 clone-history 后再 subscribe 的竞态；可以先订阅 live，再 replay 并按 seq 过滤。
- 前端 reducer 记录已消费 event id，重复 replay 不再重复追加消息、trace、tool row。
- `/jobs/{id}/state` 或新增 snapshot endpoint 返回足够恢复 UI 的状态，而不仅是 status 和 event count。

这对应 ragent 的终态协议思想，但保留 `rove` 的 event-history 优势。

### Model stream

当前 `rove` 的模型输出是：

```rust
pub struct StreamChunk {
    pub delta: String,
    pub usage: Option<Usage>,
}
```

这对早期文本流足够，但不适合长期支持：

- provider-native tool use
- thinking/reasoning channel
- tool call argument streaming
- final usage
- explicit done/error event

建议后续演进为结构化 model event，例如：

```rust
pub enum ModelEvent {
    TextDelta { text: String },
    ThinkingDelta { text: String },
    ToolUseStart { id: String, name: String },
    ToolUseDelta { id: String, args_delta: String },
    ToolUseDone { id: String, name: String, args: serde_json::Value },
    Usage { usage: Usage },
    Done,
}
```

具体枚举可以调整，但方向应该是：provider adapter 负责吃掉 OpenAI/Anthropic/Ollama 的格式差异，engine 只消费统一事件。

### Routing and first-packet probe

`rove` 已经有 `RoutingModelClient`，并且只在首包前 fallback。这个方向和 ragent 一致。

后续如果模型事件从纯文本升级到结构化事件，需要重新定义“首包成功”的语义：

- `TextDelta`、`ThinkingDelta`、`ToolUseStart` 都可以算有效首包。
- 只有 `Usage` 或 `Done` 不应算成功内容。
- provider error、timeout、empty stream 继续触发 fallback。
- 一旦任何有效 model event 被提交给 engine，就不再 fallback。

这相当于把 ragent 的 `ProbeStreamBridge` 思想搬到 Rust stream 语义下。

### Cancellation

`rove` 已经有 `CancellationToken`，比 ragent 的 Java callback 模型更适合 Rust async。但还可以借鉴 ragent 的显式 handle 思想：

- job cancel 触发 run token。
- model stream 在等待 HTTP bytes 时能尽快 abort，而不是只等 drop。
- shell/MCP/HTTP tools 在 token 取消时能 kill child process 或 abort request。
- 如果取消早于 model/tool handle 创建，后绑定时也应立即观察到 cancelled 状态。

Rust 里可以不复制 `StreamCancellationHandle` 的接口名，但需要保留“可取消资源必须有绑定点”这个设计。

## Suggested Future Phases

### Phase 1: Harden SSE replay and recovery

目标：让 Web workbench 的事件流在断线、重连、终态丢包时仍然一致。

候选改动：

- stream event envelope with `seq`
- SSE `id`
- `Last-Event-ID` or `after`
- subscribe/replay race fix
- frontend event dedupe
- full-enough job snapshot

这是最推荐先做的阶段，因为它直接修复 M6 Web UI 的可靠性边界，改动范围也相对可控。

### Phase 2: Introduce structured model events

目标：让模型层能表达 text、thinking、tool use、usage 和 done，而不是只吐文本 delta。

候选改动：

- `StreamChunk` 迁移到 `ModelEvent` 或兼容扩展结构。
- OpenAI adapter 解析 native tool calls。
- Anthropic adapter 解析 `tool_use` block。
- Ollama/OpenAI-compatible adapter 给出 best-effort 统一行为。
- engine 消费结构化 tool-use，逐步减少 JSON text parser 的角色。

这会碰核心 engine，应单独设计和测试。

### Phase 3: Unify cancellation at resource boundaries

目标：取消不只停 engine loop，也能尽快中断模型 HTTP stream、shell child process、MCP stdio/SSE request 和 hook/tool wait。

候选改动：

- 明确 run token、job token、tool token 的层级关系。
- 模型 adapter 在 select/cancel boundary 中读取 HTTP stream。
- shell/MCP child process 在取消时 kill/cleanup。
- API cancel 保证终态事件、trace、report/snapshot 只收口一次。

### Phase 4: Queue and rate-limit only when needed

目标：多用户或多 job 并发时，再考虑 ragent 的 queue limiter 思路。

当前不建议优先做 Redis queue。可以先保留简单本地并发限制，等部署形态明确后再设计。

## Proposed Decision

建议先锁定一个原则：

> `rove` 不照搬 ragent 的 chat API，但吸收它的 stream lifecycle、cancellation binding、provider-agnostic model stream 和 first-packet commit barrier 思想。

优先级建议：

1. 先做 SSE/job stream 可靠性。
2. 再做 structured model events 和 native tool-use。
3. 之后补齐取消边界。
4. 多节点队列/限流后置。

## Open Questions

进入实现设计前，需要确认：

1. SSE 阶段是否允许改变 API response shape，比如从裸 `StreamEvent` 改为 `{ seq, event }` envelope。
2. 前端是否继续使用 `EventSource`，还是因为未来 auth/header 需求提前切到 fetch SSE parser。
3. structured model event 是否要一次性替换 `StreamChunk`，还是先做兼容字段，降低 engine 改动风险。
4. `thinking` 是否作为一等 UI channel 展示，还是先只进入 trace/debug。
5. tool-use 归一化是否优先 OpenAI/Anthropic，Ollama 后续 best-effort。

## Status

本文是参考笔记，不是实施计划。下一步如果要推进，建议先把 Phase 1 写成正式设计 spec，再拆 implementation plan。

