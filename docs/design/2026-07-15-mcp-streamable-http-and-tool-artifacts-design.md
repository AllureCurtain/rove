# rove MCP Streamable HTTP and Tool Artifacts Design - 2026-07-15

> Status: **Partially implemented - remaining target is proposed**
>
> 本文保留完整目标与尚未完成的设计，不是当前实现说明。当前 MCP、Tool Result、Tool Artifact 与运行时行为仍以 [`docs/runtime/`](../runtime/README.md) 和源码为准；不得因为 Checkpoint 3/4/6 已落地，就把本文剩余的跨 transport dispatcher、session recovery 或外部互操作目标描述为已实现。

Implemented checkpoints include negotiated Streamable HTTP sessions, bounded
protocol/catalog handling, rich result envelopes, durable Tool Artifacts,
atomic `listChanged` refresh, run pinning, required/optional health, canonical
events, product diagnostics, and resume identity. Stdio and deprecated SSE keep
their compatibility adapters and registration-time catalogs; real third-party
interoperability remains an unrun optional gate.

> **Current-path correction (2026-07-26):** the modular Workspace migration
> moved MCP implementation to `runtime/src/tools/mcp_proxy.rs` and shared tool
> contracts to `core/src/tools.rs` / `core/src/types.rs`. Older `src/**` paths in
> the design-time narrative are historical; use those modular paths for future
> implementation work.

本文定义 rove 的 MCP client 与工具结果边界如何演进：在保留 stdio 的同时加入符合协议语义的 Streamable HTTP，建立统一的 JSON-RPC dispatcher、版本与 capability negotiation、session ownership、保守重试与 cancellation，并把 MCP 的 text、structured content、image、audio、resource 和 error 结果映射成可持久化、可审计、可投影的 Tool Output Envelope 与 Artifact。

设计参考 OnCall 项目已经采用的 Streamable HTTP 与多 server 管理方式，但不复制其 LangChain adapter、全局 singleton 或无差别重试策略。借鉴的是互操作方向和 interceptor boundary；rove 必须保留自己的 tool safety、approval、event、state、runtime identity 和 local-first artifact 语义。

## Suggested /goal Objective

后续进入实现阶段时，可以基于本文建立独立 `/goal`：

> Based on `docs/design/2026-07-15-mcp-streamable-http-and-tool-artifacts-design.md`, evolve rove's MCP integration into a version-negotiated multi-transport client with stdio, Streamable HTTP, and deprecated legacy SSE adapters; add a shared JSON-RPC dispatcher, explicit session ownership, capability pagination and refresh, commit-aware retry and cancellation, conservative safety metadata, typed tool result envelopes, bounded artifact persistence, canonical events, runtime identity, resume semantics, and deterministic protocol/security tests without weakening existing tool approval or workspace boundaries.

## 1. Scope and Source-of-Truth Boundary

### 1.1 本文解决什么

本文解决以下问题：

- rove 如何支持 MCP Streamable HTTP，而不把它退化成一次普通 HTTP POST；
- stdio、Streamable HTTP 与 legacy SSE 如何共享协议核心而不复制业务逻辑；
- initialize、protocol version、server capabilities、server identity 和 session 如何协商与持久化；
- JSON-RPC response、notification、server request、progress 和并发请求如何正确分发；
- `tools/list` 如何分页、响应 `listChanged`，并与当前 run 的 capability snapshot 对齐；
- request 何时可以重试、何时结果处于 `indeterminate`；
- timeout、disconnect 与 protocol cancellation 如何区分；
- MCP annotations、server trust 与 operator policy 如何共同决定安全元数据；
- text、structured content、image、audio、resource link 和 embedded resource 如何映射；
- 大 payload、敏感内容和远程 URI 如何进入本地 artifact，而不是直接进入 prompt 或 trace；
- required/optional MCP server 如何降级；
- MCP session、capability 与 artifact 身份如何进入 event、checkpoint、report 和 resume；
- CLI/API/Web 应展示什么、隐藏什么；
- 如何用 deterministic mock server 与攻击性 fixture 验收协议与安全边界。

### 1.2 本文不改变什么

本文不改变以下既有原则：

- `ToolRegistry` 仍是 runtime 可调用工具的统一入口；
- tool approval、workspace path policy、shell policy 和 mutation 记录仍由 rove 决定；
- MCP server 不能通过 schema 或 annotation 自行授予权限；
- `trace.jsonl` 是事件事实，`task_state.json` 是可恢复状态，`report.json` 是投影；
- 大型二进制内容不进入 SQLite 正文；
- provider-facing tool schema 与远端 MCP 原始 schema 之间必须有显式适配；
- 当前 `docs/runtime/` 只描述已经实现的行为；
- 本文不要求一次性实现 MCP 全部 client/server feature。

### 1.3 证据快照

本文基于 2026-07-15 的源码快照：

| 范围 | 当前证据 |
|---|---|
| rove MCP client | `runtime/src/tools/mcp_proxy.rs` |
| rove tool result | `core/src/tools.rs` |
| tool/runtime 类型 | `core/src/types.rs`、`runtime/src/foundation/types.rs` |
| MCP tests | `tests/mcp.rs`、`tests/fixtures/mcp_*.py` |
| MCP 示例配置 | `docs/examples/mcp_servers.json` |
| 当前 runtime 说明 | `docs/runtime/subsystems.md`、`docs/runtime/implementation-guide.md` |
| OnCall client | `app/agent/mcp_client.py` |
| OnCall config | `app/config.py` |
| OnCall servers | `mcp_servers/cls_server.py`、`mcp_servers/monitor_server.py` |
| 协议基线 | MCP 2025-06-18 与 2025-11-25 transport/tools specification |

官方协议链接：

- [2025-06-18 Transports](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports)
- [2025-06-18 Tools](https://modelcontextprotocol.io/specification/2025-06-18/server/tools)
- [2025-11-25 Transports](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)
- [2025-11-25 Tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools)

版本链接只是设计证据，不表示 rove 应把某个日期永久硬编码成“latest”。

## 2. Current State: rove 现在真实做了什么

### 2.1 Transport

当前 `McpTransport` 只有：

```text
Stdio
Sse
```

其中：

- stdio 启动子进程，通过 stdin/stdout 传输逐行 JSON-RPC；
- SSE 使用 GET 获取 endpoint，再向 endpoint POST JSON；
- 当前 SSE 是旧式 transport，不是 Streamable HTTP；
- 两个 adapter 分别实现 initialize、list 和 call，存在行为重复；
- protocol version 使用单个常量；
- 没有显式 transport capability 或 compatibility mode。

### 2.2 Initialize 与 server identity

当前 initialize：

- 发送 client protocol version；
- 发送空 client capabilities；
- 发送 client info；
- 接收返回值后继续 `tools/list`。

尚未形成：

- supported-version set；
- negotiated protocol version；
- server capability snapshot；
- server name/version identity；
- session ID；
- config hash；
- initialize failure classification；
- capability 与 server identity 的 runtime pin。

因此当前“初始化成功”只代表一次请求得到可解析 response，不代表后续运行有完整的协议身份。

### 2.3 Request/response dispatch

当前 stdio request 持有单个 mutex，并循环读取 stdout，直到找到 matching ID。

这在简单 mock server 下成立，但会忽略或丢弃：

- 不同 request ID 的乱序 response；
- server notifications；
- progress/logging；
- `notifications/tools/list_changed`；
- server-to-client requests；
- 并发调用；
- cancellation acknowledgement；
- malformed-but-recoverable frames 的隔离诊断。

当前 HTTP/SSE 路径也没有共享 dispatcher，因此不同 transport 可能对同一 JSON-RPC message 产生不同语义。

### 2.4 Tool discovery

当前执行一次：

```text
initialize -> tools/list -> register all returned tools
```

已具备：

- 读取 name、description、input schema；
- 把远端工具注册进本地 `ToolRegistry`；
- 为 provider 生成 `mcp__<server>__<tool>` 风格名称；
- 映射部分 annotations。

当前缺口：

- `nextCursor` pagination；
- `listChanged` notification；
- 原子 refresh；
- provider alias collision 检测；
- exact remote name 与本地 alias 的稳定映射；
- capability snapshot hash；
- 当前 run pin 与下一 run refresh 的边界；
- removed/changed tool 对已有 plan 的影响规则。

### 2.5 Tool result mapping

当前 `ToolOutput` 主要包含：

```rust
pub struct ToolOutput {
    pub content: String,
    pub mutations: Vec<ToolMutation>,
}
```

`mcp_call_result_to_text()` 主要抽取 text block。结果是：

- `isError=true` 可能退化成普通文本；
- `structuredContent` 不保留 typed identity；
- `outputSchema` 无验证；
- image/audio 不能成为一等结果；
- resource link 与 embedded resource 丢失；
- block annotations 和 `_meta` 丢失；
- unknown content type 可能被静默忽略；
- 大型 base64 或文档没有统一 artifact quota；
- prompt、UI、trace 和 report 只能共享同一个字符串投影。

### 2.6 Safety metadata

当前映射：

- `destructiveHint` -> destructive；
- `readOnlyHint` 在非 destructive 时影响 parallel-safe。

问题是：

- 缺失 annotation 时倾向非 destructive；
- 没有 `idempotentHint` 与 `openWorldHint`；
- annotation 被当作直接事实的风险较高；
- server trust、operator override 与 annotation 没有统一决策表；
- remote schema 不能表达 rove workspace、approval 和 egress policy；
- 同名工具或配置变化后的安全身份不稳定。

MCP annotations 是 hints，不是授权。尤其面对不可信 server，缺失 hint 必须按保守语义处理。

### 2.7 Failure 与 lifecycle

当前已有：

- per-request timeout；
- stdio stderr 有界捕获；
- JSON-RPC error 转换；
- client drop 时清理 child process；
- server/tool 注册错误可见。

当前缺少：

- connect/initialize/request/idle 分层 timeout；
- request commit point；
- post-commit unknown outcome；
- protocol cancellation；
- session close；
- reconnect/reinitialize state machine；
- required 与 optional server；
- per-server degraded health；
- retry safety classification；
- process restart 后的明确 session 失效语义。

当前某个 server 注册失败可以使整个 runtime tool registry 构建失败。这不适合包含多个可选外部 server 的 AgentDefinition。

### 2.8 Test coverage

当前 `tests/mcp.rs` 主要覆盖 stdio：

- mock server 注册与调用；
- timeout；
- JSON-RPC error；
- child cleanup；
- opt-in official filesystem server smoke。

尚未覆盖：

- legacy SSE contract；
- Streamable HTTP JSON response；
- Streamable HTTP SSE response；
- session header、404 与 DELETE；
- GET notification stream；
- pagination/listChanged；
- out-of-order response；
- cancellation；
- structured/multimodal result；
- annotation conservative defaults；
- artifact attack surface；
- retry/indeterminate；
- optional server degradation。

## 3. OnCall: 可借鉴与不可照搬

### 3.1 可借鉴

OnCall 已经使用：

```text
transport = "streamable-http"
http://localhost:8003/mcp
http://localhost:8004/mcp
```

其 server 通过 FastMCP 启动 Streamable HTTP，client 使用 `MultiServerMCPClient`。值得借鉴：

- 以 Streamable HTTP 作为现代远程 transport；
- 多 server 统一管理；
- lazy connection，避免启动时无条件建立所有远程连接；
- interceptor 作为 timeout/retry/telemetry 的统一边界；
- `CallToolResult(isError=True)` 保留工具级失败；
- server 与 business agent 分进程，边界清晰。

### 3.2 不可照搬

OnCall 当前做法不适合作为 rove runtime contract：

- 全局 singleton 可能跨 run 复用有状态 session；
- adapter 隐藏 protocol negotiation、session、dispatcher 和 content mapping；
- interceptor 对 exception 无差别重试，可能重复 destructive 或 non-idempotent 调用；
- 多次失败后用文本 error 返回，transport error 与 tool error 混合；
- 没有 rove 所需的 runtime identity、checkpoint、event 和 resume 语义；
- 没有 artifact/provenance/schema validation；
- server decorator 记录完整 kwargs，可能泄漏敏感参数；
- config 与日志的 secret redaction 边界不完整；
- server/tool 的 required/optional 与 trust 没有显式声明。

### 3.3 借鉴结论

rove 应借鉴：

> Streamable HTTP + multi-server manager + interceptor/dispatcher boundary。

rove 不应借鉴：

> 全局隐式生命周期 + blind retry + adapter 内部黑盒协议状态 + text-only error/result。

## 4. Protocol Baseline

### 4.1 版本协商

设计必须区分：

- client 支持的 protocol versions；
- initialize request 提议版本；
- server 返回版本；
- 最终 negotiated version；
- version-specific feature gates；
- unsupported version failure。

目标状态示例：

```rust
struct McpProtocolSupport {
    preferred: ProtocolVersion,
    supported: Vec<ProtocolVersion>,
}

struct McpNegotiatedIdentity {
    protocol_version: ProtocolVersion,
    server_info: ServerInfo,
    capabilities: ServerCapabilities,
}
```

规则：

1. client 使用明确支持集合，而不是追随网页“latest”；
2. server 返回版本必须属于支持集合；
3. negotiated version 进入 runtime identity；
4. feature 使用由 negotiated version 与 capability 共同决定；
5. 未协商成功前不得调用 `tools/list`；
6. resume 不得把旧 session 的 negotiated identity 假装为当前连接身份。

### 4.2 Streamable HTTP 基本语义

一个 endpoint 支持：

- POST：client-to-server JSON-RPC；
- GET：可选 server-to-client SSE stream；
- DELETE：可选 session termination。

POST 至少声明：

```http
Accept: application/json, text/event-stream
Content-Type: application/json
MCP-Protocol-Version: <negotiated-version>
Mcp-Session-Id: <session-id-if-issued>
```

client 必须同时支持：

- 单个 JSON response；
- SSE stream 中的 notification/server request/最终 response；
- accepted/no-body 响应；
- HTTP status 与 JSON-RPC error 的分层。

### 4.3 Session

server 可以在 initialize response 返回 `Mcp-Session-Id`。

规则：

- session ID 视为敏感 bearer-like value；
- 后续同 session 请求必须携带；
- session 404/明确 invalid 时重新 initialize；
- client 正常结束时尝试 DELETE；
- 405 表示 server 不支持 DELETE，不应当作 run failure；
- session ID 原值不进入普通 trace、report 或 UI；
- session 不跨不相容的 config hash/server identity 复用。

### 4.4 SSE 与 resumability

Streamable HTTP 中 SSE 不是旧 transport 的同义词：

- POST response 可以自身成为 SSE stream；
- GET 可以建立独立 server-to-client stream；
- event ID 是 per-stream cursor；
- reconnect 可以携带 `Last-Event-ID`；
- transport resume 不等于 runtime run resume；
- network disconnect 不等于远端 tool cancellation。

### 4.5 Tools protocol

client 必须理解：

- `tools/list` cursor pagination；
- `notifications/tools/list_changed`；
- tool `title`、`description`、`inputSchema`、`outputSchema`；
- `readOnlyHint`、`destructiveHint`、`idempotentHint`、`openWorldHint`；
- call result `content[]`、`structuredContent`、`isError`；
- text、image、audio、resource link、embedded resource；
- unknown future block 的兼容保存。

## 5. Design Goals

1. 支持真实 Streamable HTTP，而不是只支持 HTTP URL 配置。
2. 三种 transport 共享 initialize、dispatcher、tools 和 result mapping。
3. protocol/session/capability identity 可观测、可固定、可诊断。
4. 并发 response、notification 与 server request 不被误丢弃。
5. tool list 能分页、刷新并保持 run 内 snapshot 稳定。
6. timeout、retry、cancel 与 unknown outcome 有严格语义。
7. annotations 缺失或不可信时采用 conservative defaults。
8. MCP result 成为 typed envelope，不再只能压成字符串。
9. 大型或二进制 content 进入有界 artifact store。
10. model、UI、report、audit 使用不同安全投影。
11. optional server 失败可显式降级，required server 失败可阻止启动。
12. state/checkpoint/report 能解释使用了哪个 server、协议、capability 与 artifact。
13. 所有关键路径有 deterministic contract/security tests。
14. 迁移期间保留 stdio 和 legacy SSE 兼容。

## 6. Non-Goals

本文不要求：

- rove 实现通用 MCP server；
- 第一阶段支持 MCP 的 prompts、sampling、roots、elicitation 全部 feature；
- 跨进程恢复旧 HTTP session；
- 自动下载所有 resource link；
- 在 prompt 中内联 image/audio/base64；
- 信任 server 提供的 filename、MIME、URI 或 annotation；
- 对未知写操作自动 exactly-once；
- 让 MCP permission 绕过 rove approval；
- 一次性替换现有 `Tool` trait；
- 把 artifact storage 变成通用云对象存储；
- 为所有 server 默认开启公网 egress；
- 在实现前修改 `docs/runtime/implementation-status.md`。

## 7. Design Principles

### 7.1 Protocol truth before convenience

HTTP library、MCP SDK 或 adapter 只是实现手段。session、version、message ordering、cancellation 和 result type 必须由 rove contract 明确表达。

### 7.2 Connection identity is not capability identity

同一个 URL 重连后可能得到不同 server version、capabilities 或 tools。连接复用不能替代 capability snapshot。

### 7.3 Disconnect is not cancellation

丢失 response 后，远端操作可能已完成。不得把 timeout 简化成“没执行”。

### 7.4 Remote metadata is untrusted input

schema、annotation、URI、MIME、title、filename 和 `_meta` 都需要大小限制、验证、redaction 与 operator policy。

### 7.5 Preserve rich data, project narrowly

原始结果应有界保存；不同消费者只得到最小安全投影。不能因为 provider 只吃文本就永久丢掉结构化证据。

### 7.6 Pin facts within a run

当前 run 使用固定的 server/capability/profile identity。动态变化进入事件和下一次决策，不能静默改写已开始的 plan。

### 7.7 Safe degradation is typed

`unavailable`、`degraded`、`error`、`partial`、`indeterminate` 不是同一个字符串。

## 8. Target Architecture

```text
AgentRuntimeProfile / Tool policy
                |
                v
        McpServerManager
        |       |       |
      stdio  streamable_http  legacy_sse
        \       |       /
          McpClientCore
          - initialize/version/session
          - JSON-RPC dispatcher
          - capability catalog
          - request lifecycle
          - cancellation/retry
                  |
                  v
       McpToolAdapter / ToolRegistry
                  |
                  v
       ToolOutputEnvelope builder
          |          |          |
       summary   structured   artifacts
          |          |          |
       model      report       store/UI
```

核心组件：

| 组件 | 责任 |
|---|---|
| `McpServerManager` | 解析配置、管理 server health/connection scope、required/optional、refresh |
| `McpClientCore` | 协议状态、request ID、dispatcher、capabilities、session |
| `McpTransportAdapter` | byte/message I/O，不决定 tool safety |
| `McpCapabilityCatalog` | exact remote identity、pagination、snapshot、alias |
| `McpToolAdapter` | 把本地 Tool call 映射为 `tools/call` |
| `ToolOutputEnvelopeBuilder` | 验证/分类 result、生成 summary/blocks/artifacts |
| `ToolArtifactStore` | quota、hash、snapshot、metadata、retention |
| `McpEventProjector` | 把内部状态转成 canonical safe events |

## 9. Server Configuration and Identity

### 9.1 Target config

概念配置：

```toml
[[mcp.servers]]
id = "monitoring-primary"
transport = "streamable_http"
url = "https://mcp.internal.example/mcp"
required = false
trust = "organization"
session_scope = "run"
protocol_versions = ["2025-11-25", "2025-06-18"]
connect_timeout_ms = 5000
request_timeout_ms = 30000
idle_timeout_ms = 60000
max_response_bytes = 4194304
max_artifact_bytes = 16777216
headers_env = "ROVE_MCP_MONITOR_HEADERS_JSON"
retry_policy = "safe_default"

[mcp.servers.egress]
allowed_hosts = ["mcp.internal.example"]
allow_redirects = false
require_https = true
```

stdio 保留：

```toml
[[mcp.servers]]
id = "filesystem-local"
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "."]
required = false
trust = "local_process"
session_scope = "run"
env_allowlist = ["PATH"]
```

### 9.2 Secret references

- config 只保存 env/secret reference；
- dump-config 只显示 reference name 与 present/missing；
- header value、authorization、session ID 不进入普通日志；
- stdio env 默认 allowlist，不自动复制整个 parent environment；
- URL userinfo 被拒绝；
- query 中疑似 token 的 URL 在事件中 redacted；
- error body 按大小与 secret pattern 截断。

### 9.3 Stable identity

建议身份：

```text
server_config_id      operator-defined stable ID
server_config_hash    redacted canonical config hash
connection_id         one live connection
session_hash          hash only, when session exists
server_identity       negotiated name/version
capability_hash       canonical tools/capabilities hash
remote_tool_name      exact MCP name
local_capability_id   stable rove capability ID
provider_alias        collision-resistant model-facing name
```

provider alias 不能只依赖字符替换。建议：

```text
mcp__<safe-server-prefix>__<safe-tool-prefix>__<short-hash>
```

映射表进入 runtime profile，不要求模型猜 remote name。

### 9.4 Collision rule

发生以下情况时注册失败或显式隔离：

- 两个 server config ID 相同；
- 同一 snapshot 中两个 remote tool 生成同一 alias；
- 同 capability ID 绑定到不兼容 schema；
- refresh 后相同 remote identity 的 input/output schema 非兼容变化；
- hash 计算缺少影响安全语义的字段。

## 10. Protocol and Session State Machine

目标状态：

```text
Configured
  -> Connecting
  -> Initializing
  -> Ready
  -> Degraded
  -> Reconnecting
  -> Closing
  -> Closed

terminal:
  -> FailedRequired
  -> DisabledByPolicy
```

### 10.1 Connecting

- transport adapter 建立 I/O；
- 应用 TLS、host、redirect、process env policy；
- 只记录 safe endpoint identity；
- connect timeout 不与 request timeout 混用。

### 10.2 Initializing

- 分配 initialize request ID；
- 发送 preferred protocol 与 client capabilities；
- 验证 JSON-RPC envelope；
- 验证 negotiated version；
- 记录 serverInfo/capabilities；
- 获取并保护 session ID；
- 发送 initialized notification；
- 之后才允许 list/call。

### 10.3 Ready

Ready 表示：

- transport 可用；
- initialization 完成；
- protocol identity 固定；
- initial tool catalog 成功或按配置显式 degraded；
- dispatcher 已运行；
- health 与 last activity 可观测。

### 10.4 Reconnecting

触发条件：

- transport EOF；
- session 404；
- retryable pre-commit connection error；
- GET SSE stream 中断；
- operator refresh。

reconnect 后：

- 新 connection ID；
- 必要时重新 initialize；
- 比较 server/capability identity；
- 不复用旧 session 原值；
- 不自动重放 indeterminate write；
- capability 变化发事件。

### 10.5 Closing

- 停止接受新请求；
- 对 active request 发 cancellation 或等待 grace period；
- Streamable HTTP 尝试 DELETE session；
- stdio 关闭 stdin，等待后终止 child；
- flush dispatcher diagnostics；
- 释放 secret-bearing memory。

## 11. Transport Adapters

### 11.1 Shared boundary

transport adapter 只负责可靠地收发 protocol message：

```rust
trait McpTransportAdapter {
    async fn connect(&mut self) -> Result<TransportConnection>;
    async fn send(&self, message: JsonRpcMessage) -> Result<SendReceipt>;
    async fn next_inbound(&self) -> Result<InboundFrame>;
    async fn close(&self, reason: CloseReason) -> Result<()>;
    fn transport_kind(&self) -> McpTransportKind;
}
```

adapter 不负责：

- 决定 tool 是否 destructive；
- 将 result 投影给 model；
- 选择 retry；
- 修改 plan；
- 生成 provider-facing alias；
- 把 HTTP success 当作 tool success。

`SendReceipt` 至少表达：

```text
NotSent
HeadersCommitted
BodyPartiallySent
BodyCommitted
```

它是 retry/indeterminate 判断的输入，不承诺底层能精确获知 server 是否开始执行。

### 11.2 stdio

stdio 目标改造：

- 独立 reader task 持续解析 frame；
- writer 使用有界 queue；
- dispatcher 按 ID 路由 response；
- notification/server request 不再因 ID 不匹配而丢弃；
- stdout 只接受 protocol frame；
- stderr 独立有界捕获；
- invalid JSON 有计数与阈值；
- child exit 与 active request 同步失败；
- drop/close 走明确 cleanup；
- command、args、cwd、env 进入 redacted config hash。

stdio 不支持 HTTP session header，但仍使用相同 initialize、capability、request 和 cancellation state。

### 11.3 Streamable HTTP

Streamable HTTP adapter 至少支持：

1. initialize POST；
2. 普通 POST + JSON response；
3. POST + SSE response；
4. accepted/no-body；
5. 可选 GET notification stream；
6. session/version headers；
7. optional DELETE；
8. `Last-Event-ID` reconnect；
9. response body 与 event 大小限制；
10. HTTP status/headers 的安全诊断。

HTTP client policy：

- 默认只允许 HTTPS；
- loopback/local test 可显式允许 HTTP；
- 默认不跟随 redirect；
- 若允许 redirect，重新检查 scheme/host/port；
- 禁止 URL userinfo；
- host allowlist 在 DNS 前后都检查；
- 防止 DNS rebinding 与 private-network 越界；
- TLS validation 默认开启；
- proxy 使用必须显式；
- decompressed size 也受限，防止压缩炸弹；
- response content type 不匹配时失败，不猜测。

### 11.4 Legacy SSE

现有 SSE 暂时作为：

```text
transport = "legacy_sse"
deprecated = true
```

规则：

- 与 `streamable_http` 名称严格区分；
- dump-config/diagnostics 显示 deprecated；
- 共享 dispatcher 和 result mapping；
- 不伪造 session/DELETE/POST-SSE 能力；
- 有独立 contract tests；
- 给出迁移提示；
- 只有确认使用方迁移后才能删除。

### 11.5 Transport feature matrix

| Feature | stdio | Streamable HTTP | legacy SSE |
|---|---:|---:|---:|
| initialize negotiation | 是 | 是 | 是 |
| concurrent request IDs | 目标支持 | 目标支持 | 受 server 约束 |
| server notifications | 是 | 是 | 是 |
| HTTP session ID | 不适用 | 是 | 通常否 |
| GET stream | 不适用 | 可选 | 是 |
| POST SSE response | 不适用 | 是 | 否 |
| DELETE session | 不适用 | 可选 | 否 |
| Last-Event-ID | 不适用 | 可选 | adapter-specific |
| child lifecycle | 是 | 否 | 否 |

## 12. Shared JSON-RPC Dispatcher

### 12.1 Message classes

dispatcher 必须区分：

```text
Response(id, result)
ErrorResponse(id, error)
Notification(method, params)
ServerRequest(id, method, params)
InvalidFrame(reason, bounded_excerpt)
```

不能用“是否有 id”之外的模糊字符串判断。

### 12.2 Pending request table

```rust
struct PendingRequest {
    request_id: JsonRpcId,
    method: String,
    sent_at: Instant,
    deadline: Instant,
    commit_state: CommitState,
    safety: RequestSafety,
    completion: oneshot::Sender<RequestOutcome>,
}
```

规则：

- ID 在 connection 内唯一；
- pending table 有最大容量；
- duplicate response 发 protocol diagnostic；
- unknown response ID 有界记录；
- timeout 从 pending table 原子移除；
- late response 不重新完成已超时 request；
- close 时所有 pending request 得到 typed outcome；
- response 内容先过总大小限制，再进入 method-specific parser。

### 12.3 Notifications

首批处理：

- `notifications/tools/list_changed`；
- progress；
- logging/message；
- cancelled/related lifecycle notification；
- unknown notification。

unknown notification：

- 不导致 connection 立即失败；
- 方法名、大小和计数有界记录；
- payload 默认不进入用户报告；
- 达到 abuse 阈值可以降级/断开。

### 12.4 Server requests

首阶段 rove 可以不实现所有 server request，但必须返回规范化 method-not-supported，而不是丢弃。

处理策略：

| 类别 | 行为 |
|---|---|
| 明确支持且 policy 允许 | 路由到受限 handler |
| 协议已知但未实现 | JSON-RPC method not supported |
| 未知方法 | method not found / supported error |
| 涉及 sampling/elicitation | 默认拒绝，未来独立设计 |
| payload 超限 | protocol error + health penalty |

server request 不能借 MCP connection 反向获得 unrestricted local tool access。

### 12.5 Ordering

- 同 request 的 SSE messages 按 stream 顺序处理；
- 不同 request 不假设全局顺序；
- progress 可在 terminal response 前到达；
- terminal response 后的 progress 作为 late diagnostic；
- `listChanged` 与 active tool call 并发时不撤销已 pin binding；
- event sequence 使用 rove canonical event seq，不把远程 event ID 当本地 seq。

## 13. Capability Discovery and Refresh

### 13.1 Pagination

`tools/list` 必须循环处理 cursor：

```text
cursor = none
repeat:
  page = tools/list(cursor)
  validate page
  append tools
  cursor = page.nextCursor
until cursor is none
```

保护：

- 最大 page 数；
- 最大 tool 数；
- 最大 schema bytes；
- cursor 循环检测；
- duplicate remote name；
- 每页与总请求 timeout；
- 原子提交 catalog。

任一页失败时不能把半个 catalog 当完整 catalog；可保留上一个已验证 snapshot，并标记 refresh degraded。

### 13.2 Normalized descriptor

```rust
struct McpToolDescriptor {
    server_config_id: String,
    server_identity: ServerIdentity,
    remote_name: String,
    title: Option<String>,
    description: Option<String>,
    input_schema: JsonSchema,
    output_schema: Option<JsonSchema>,
    annotations: McpToolAnnotations,
    raw_descriptor_hash: ContentHash,
}
```

本地 capability descriptor 额外包含：

- stable capability ID；
- provider alias；
- trust decision；
- operator overrides；
- effective safety；
- schema compatibility status；
- availability/degradation；
- snapshot hash。

### 13.3 listChanged

收到 `notifications/tools/list_changed`：

1. 发 `mcp_capabilities_change_detected`；
2. debounce；
3. 后台构建完整新 catalog；
4. 验证后原子发布；
5. 计算 added/removed/changed；
6. 发 safe diff event；
7. 新 run 使用新 snapshot；
8. active run 默认继续使用 pinned snapshot。

如果 active binding 已被 server 删除：

- 尚未调用：在执行点返回 capability unavailable，交给 PlanEvaluator；
- 正在调用：按该调用真实结果完成；
- 已完成：ledger 不变；
- 不用同名新 schema 静默替换。

### 13.4 Snapshot pinning

`McpCapabilitySnapshot` 至少包含：

```text
snapshot_id
created_at
server_config_hash
server_identity
protocol_version
server_capabilities_hash
tool_descriptors[]
catalog_hash
```

它与第二篇 [Agent Definition and Procedural Knowledge Design](2026-07-14-agent-definition-and-procedural-knowledge-design.md) 的 capability binding 对齐：

- AgentDefinition 引用 stable capability ID；
- runtime profile 解析到 snapshot 中的具体 remote tool；
- plan 固定 snapshot signature；
- approval 仍基于 effective local policy；
- refresh 不产生额外授权。

## 14. Tool Invocation Lifecycle

目标调用状态：

```text
Prepared
  -> AwaitingApproval
  -> Dispatching
  -> InFlight
  -> Receiving
  -> Validating
  -> PersistingArtifacts
  -> Completed

terminal alternatives:
  Rejected
  Cancelled
  TimedOutKnownNotSent
  Error
  Partial
  Indeterminate
```

### 14.1 Prepared

- 解析 local alias；
- 验证 pinned descriptor；
- 验证 JSON arguments；
- 应用 workspace/capability/operator policy；
- 计算 effective safety；
- 生成 call ID；
- 估算 payload；
- 确认 server health。

### 14.2 AwaitingApproval

approval 展示：

- 本地 capability 名称；
- server safe identity；
- 操作摘要；
- destructive/open-world/unknown 风险；
- arguments 的 redacted projection；
- 是否可能产生外部副作用。

approval 不展示：

- authorization header；
- session ID；
- secret args；
- raw binary；
- 未经验证的 server HTML/Markdown。

### 14.3 Dispatching/InFlight

- 注册 pending request；
- 发送 `tools/call`；
- 记录 commit state；
- 发 `tool_call_started` 与 MCP-specific metadata；
- 接收 progress；
- 保持 cancel handle；
- timeout 后进入 cancel/indeterminate 判定。

### 14.4 Validating

依次验证：

1. JSON-RPC envelope；
2. method result shape；
3. `isError`；
4. content block type；
5. total/block size；
6. base64；
7. URI/MIME/name；
8. `structuredContent` 对 `outputSchema`；
9. artifact policy；
10. projection safety。

### 14.5 Completion

只有 envelope、status 与必要持久化完成后，才产生 terminal tool event。

`isError=true` 不得产生 `tool_call_completed(success=true)`。artifact 部分成功但某块被拒绝时，根据 policy 返回 `Partial` 或 `Error`，不能静默成功。

## 15. Retry, Commit Point, and Indeterminate Outcomes

### 15.1 Request safety

综合来源：

```text
operator override
  > trusted local capability policy
  > AgentRuntimeProfile restriction
  > server annotations as hints
  > conservative default
```

有效分类：

```text
ReadOnlyIdempotent
ReadOnlyUnknownIdempotency
MutatingIdempotent
MutatingNonIdempotent
Unknown
```

### 15.2 Retry matrix

| Failure point | Read-only/idempotent | Mutating/unknown |
|---|---|---|
| DNS/connect before send | 可按预算重试 | 可按预算重试 |
| request 明确未发送 | 可重试 | 可重试 |
| headers/body 已 commit，未收到 response | policy 允许时重试或查询 | 不自动重试 |
| server 429/503 且明确未执行 | 尊重 Retry-After | 仅有明确 non-execution 证据时 |
| JSON-RPC tool error | 默认不 transport retry | 默认不 retry |
| `isError=true` | 交给 Agent/plan | 交给 Agent/plan |
| schema/artifact validation error | 不重放 | 不重放 |
| session 404 | reinitialize 后按安全性决定 | 结果未知时不重放 |

### 15.3 Indeterminate

当请求可能已被 server 接收/执行，但 terminal response 丢失：

```rust
ToolExecutionStatus::Indeterminate {
    reason,
    request_id_hash,
    server_identity,
    last_known_phase,
    possible_external_effects,
}
```

规则：

- 不把它记为 failed-and-safe-to-retry；
- 进入 StepRecord 与 checkpoint；
- Finalizer 必须告知用户“执行结果未知”；
- 后续 plan 可选择只读验证；
- destructive call 默认要求 operator 决策；
- resume 不自动重放；
- 若 server 支持业务 idempotency key/query，可由 capability-specific recovery 使用；
- runtime 本身不声称 exactly-once。

### 15.4 Retry budget

retry 计入：

- per-request attempt；
- server retry budget；
- run tool-call budget；
- elapsed-time budget；
- event/trace。

backoff：

- exponential + jitter；
- 有最大值；
- 尊重安全的 Retry-After；
- cancellation 立即终止 backoff；
- 不跨 run 隐式持续重试。

## 16. Cancellation, Timeout, and Cleanup

### 16.1 Timeout layers

| Timeout | 含义 |
|---|---|
| connect | 建立 process/HTTP connection |
| initialize | 完成 protocol handshake |
| request | 一个 JSON-RPC request 到 terminal response |
| idle | SSE/connection 无任何活动 |
| progress stall | 有长任务 capability 时，无 progress 的阈值 |
| close grace | cancellation/DELETE/child exit 等待 |

timeout 发生必须记录具体层，不能只写 `MCP timeout`。

### 16.2 Cancellation

- local run cancel 触发 active call cancellation；
- 如果协议支持，发送 MCP cancelled notification；
- notification 本身失败不改变“本地已取消等待”；
- transport future drop 不是远端取消证据；
- cancel 后收到 terminal response，保存为 late outcome diagnostic；
- 若 mutating request 已 commit 且无结果，状态为 indeterminate；
- stdio child 不因单次 call cancel 默认整体 kill，除非 server policy 只支持 process abort；
- run 结束时按 server/session scope 清理。

### 16.3 Cleanup

正常：

- 停止新调用；
- 完成或取消 active calls；
- DELETE HTTP session；
- 关闭 GET stream；
- 关闭 stdio；
- flush artifact metadata；
- 记录 close outcome。

异常：

- 有界等待；
- kill child process tree；
- abort I/O tasks；
- pending request 全部 typed complete；
- 不在 destructor 中阻塞无限时间。

## 17. Trust, Annotations, and Effective Safety

### 17.1 Trust levels

建议：

```text
untrusted_remote
organization
local_process
operator_pinned
```

trust 影响：

- 是否允许连接；
- annotation 权重；
- resource fetch；
- artifact MIME/URI 处理；
- session sharing；
- retry；
- telemetry detail。

trust 不影响：

- 绕过 approval；
- 越过 workspace；
- 读取未授权 secret；
- 自动执行 destructive 工具。

### 17.2 Conservative defaults

annotation 缺失或 invalid 时：

```text
read_only = false
destructive = true or unknown-high-risk
idempotent = false/unknown
open_world = true/unknown
parallel_safe = false
approval = required unless stricter policy denies
```

这比当前“缺失 destructiveHint 即非 destructive”更保守。

### 17.3 Effective policy

```rust
struct EffectiveToolSafety {
    read_only: Decision<bool>,
    destructive: Decision<bool>,
    idempotent: Decision<bool>,
    open_world: Decision<bool>,
    parallel_safe: bool,
    approval: ApprovalRequirement,
    reasons: Vec<PolicyReason>,
}
```

`Decision` 记录：

- value；
- source；
- confidence/trust；
- override；
- conflict。

server annotation 与 operator policy 冲突时，采取更严格结果并产生诊断。

### 17.4 Schema is not a sandbox

JSON schema 只验证形状。即使参数字段名是 `path`、`url` 或 `query`：

- MCP server 在独立信任域执行；
- rove 不能假设它遵守本地 path policy；
- egress/tool allowlist 决定是否允许调用；
- description 中的“safe/read-only”不是安全证明；
- remote tool output 仍是不可信数据。

## 18. Tool Output Envelope

### 18.1 Target model

```rust
struct ToolOutputEnvelope {
    status: ToolExecutionStatus,
    summary_text: String,
    content_blocks: Vec<ToolContentBlock>,
    structured_content: Option<StructuredToolContent>,
    artifacts: Vec<ToolArtifactRef>,
    mutations: Vec<ToolMutation>,
    external_effects: Vec<ExternalEffect>,
    protocol_metadata: ToolProtocolMetadata,
    diagnostics: Vec<ToolDiagnostic>,
}
```

兼容策略：

- 现有 `ToolOutput.content` 可暂时由 `summary_text` 投影；
- 现有 `mutations` 原样保留并扩展；
- 新 envelope 先进入内部 runtime，再逐步暴露 API/Web；
- provider adapter 不直接看到 raw envelope；
- non-MCP tool 也可逐步使用同一 envelope。

### 18.2 Status taxonomy

```rust
enum ToolExecutionStatus {
    Success,
    Partial,
    Error,
    Rejected,
    Cancelled,
    TimedOutKnownNotSent,
    Indeterminate,
}
```

Error detail 分类：

- transport；
- HTTP；
- JSON-RPC；
- protocol；
- remote tool `isError`；
- input schema；
- output schema；
- artifact；
- policy；
- internal。

### 18.3 Content blocks

```rust
enum ToolContentBlock {
    Text(TextBlock),
    Image(ArtifactBackedBlock),
    Audio(ArtifactBackedBlock),
    ResourceLink(ResourceLinkBlock),
    EmbeddedResource(EmbeddedResourceBlock),
    Unknown(UnknownBlockRef),
}
```

每块保留：

- ordinal；
- source type；
- audience/priority annotation（经验证）；
- MIME；
- bounded inline preview；
- artifact reference；
- provenance；
- validation status。

### 18.4 Protocol metadata

只保存安全元数据：

```text
protocol = "mcp"
server_config_id
server_identity_hash/display-safe identity
protocol_version
capability_snapshot_id
remote_tool_name
request_id_hash
connection_id
session_hash
attempt_count
timing
```

不保存普通可读 session ID、Authorization、raw headers。

## 19. MCP Content Mapping

### 19.1 Text

- 小文本可 inline；
- 统一 Unicode/size validation；
- 保留原始文本 hash；
- 超过 prompt inline limit 时完整内容进入 artifact；
- summary 使用 deterministic truncation 或显式 summarizer；
- 不把文本中的指令提升为 system/workspace policy；
- UI 默认以不执行 HTML 的纯文本/安全 Markdown 展示。

### 19.2 Image

- 验证 base64；
- 解码后按 byte quota；
- sniff 与 declared MIME 对比；
- 保存 content hash；
- 默认不把 base64 写入 trace/report；
- model 只在 provider 支持、Agent policy 允许时得到 image reference/projection；
- SVG/HTML 等 active content 需要更严格策略；
- 下载使用 safe Content-Disposition。

### 19.3 Audio

- 与 image 相同的 byte/hash/quota 原则；
- 不默认播放或转写；
- duration/codec metadata 只能作为非权威信息；
- 需要转写时是另一个显式 tool/capability；
- prompt 只得到 artifact ref 与安全摘要。

### 19.4 Resource link

resource link 默认 lazy：

- 保存 URI 字符串与 server provenance；
- 不自动 fetch；
- fetch 需要单独 capability/policy/budget；
- remote `file://` 是 server-scoped URI，不是 rove 本地路径；
- URL 重新进行 scheme/host/egress 验证；
- link title/name 不用作本地 filename；
- link 内容变化不能改变原 tool result 的 snapshot identity。

### 19.5 Embedded resource

- 验证 text/blob；
- 保存原 URI 与 MIME 作为不可信 metadata；
- 内容进入 artifact；
- hash 固定；
- 大文本不直接进 prompt；
- provenance 绑定 server/session/call；
- 若 URI 与已有 artifact 冲突，以 content hash 和 call identity 区分。

### 19.6 Unknown block

未来 block type：

- 在总大小限制内把 canonical raw JSON 保存为 artifact；
- 生成 `unsupported content type` diagnostic；
- 不静默丢弃；
- 不把 raw JSON 直接注入 model；
- policy 可以把关键 unknown block 提升为 Partial/Error；
- telemetry 记录 type name，不记录敏感 payload。

## 20. Structured Content, Schema, and Error Semantics

### 20.1 structuredContent

`StructuredToolContent`：

```rust
struct StructuredToolContent {
    value: serde_json::Value,
    canonical_hash: ContentHash,
    schema_hash: Option<ContentHash>,
    validation: SchemaValidationStatus,
    artifact_ref: Option<ToolArtifactRef>,
}
```

规则：

- 保留原始 JSON 语义；
- canonicalize 仅用于 hash，不改写展示值；
- 大对象外置 artifact；
- model projection 有字段/深度/字符上限；
- sensitive key redaction；
- schema validation 结果显式；
- UI 可结构化展示，但不执行内容。

### 20.2 outputSchema

tool descriptor 有 `outputSchema` 时：

- catalog 阶段验证 schema 自身；
- call result 的 `structuredContent` 必须验证；
- schema 不支持的 keyword 有明确兼容策略；
- validation failure 默认 Tool Error 或 Partial，不伪装 success；
- raw result 可按 policy 保存用于审计；
- schema 变化导致 capability hash 变化；
- model 看到“schema validation failed”，不能被失败数据误导。

没有 outputSchema：

- structured content 仍可保留；
- validation = `NoSchema`；
- 不据此提高 trust。

### 20.3 `isError`

`isError=true` 表示 MCP tool execution error：

- transport/JSON-RPC 本身可能成功；
- envelope status = Error；
- content blocks 可作为错误证据保存；
- ToolCallFailed event；
- retry 由 Agent/plan 与安全策略决定，不由 HTTP 自动重试；
- Finalizer 可以引用错误摘要；
- 不计为 successful mutation。

### 20.4 Partial

示例：

- text 成功，image 超过 quota；
- structured content valid，resource URI 被 policy 拒绝；
- 一部分 embedded resources 保存失败；
- server 返回 success，但某 unknown critical block 无法解释。

Partial 必须包含：

- 可用内容；
- rejected block 列表；
- 是否影响任务结论；
- 是否允许继续；
- artifact cleanup 状态。

### 20.5 Error precedence

由外到内：

```text
policy/connection
  -> HTTP/transport
  -> JSON-RPC
  -> MCP result shape
  -> isError
  -> schema/content validation
  -> artifact persistence
  -> projection
```

不能用内层成功覆盖外层失败。例如 HTTP 200 中 invalid JSON 不是成功；`isError=false` 但 artifact policy 拒绝全部关键内容也不是完整成功。

## 21. Tool Artifact Model and Persistence

### 21.1 Artifact reference

```rust
struct ToolArtifactRef {
    artifact_id: ArtifactId,
    kind: ToolArtifactKind,
    mime_type: Option<String>,
    byte_length: u64,
    sha256: String,
    storage_ref: String,
    source: ToolArtifactSource,
    original_uri: Option<String>,
    audience: Option<Vec<String>>,
    priority: Option<f32>,
    last_modified: Option<String>,
    sensitivity: Sensitivity,
    trust: ArtifactTrust,
    validation: ArtifactValidation,
}
```

`ToolArtifactSource` 至少包含：

- run ID；
- tool call ID；
- server config ID；
- server identity hash；
- connection/session hash；
- remote tool name；
- content block ordinal；
- captured timestamp。

### 21.2 Storage layout

目标：

```text
.rove/runs/<run_id>/
  artifacts/
    <artifact_id>/
      payload
      metadata.json
  tool_artifacts.jsonl
  trace.jsonl
  task_state.json
  report.json
```

原则：

- payload 使用 content hash/opaque ID，不信任 remote filename；
- metadata 原子写；
- JSONL 记录 append-only creation/rejection；
- SQLite 只索引 ID、hash、kind、size、run 与 storage ref；
- report 引用 artifact ID，不复制大正文；
- task state 保存恢复必要 pointer；
- 临时文件在成功 commit 或失败 cleanup 后不残留。

### 21.3 Quotas

至少有：

```text
max_inline_text_bytes
max_content_blocks
max_single_artifact_bytes
max_tool_call_artifact_bytes
max_run_artifact_bytes
max_structured_json_depth
max_structured_json_nodes
max_unknown_block_bytes
```

quota 发生时：

- 发 artifact rejected event；
- 保存 bounded metadata；
- 决定 Partial/Error；
- 不继续无界读取；
- cleanup partial payload；
- 计入 server health/diagnostics。

### 21.4 Deduplication

- 同一 run 内可以按 hash 复用 payload；
- metadata/provenance 仍保留每次 call reference；
- 跨 run dedupe 是未来优化，不应破坏 retention/sensitivity；
- hash 在验证后的 bytes 上计算；
- secret/sensitive artifact 不因 dedupe 暴露存在性。

### 21.5 Retention

artifact retention 与 run retention 对齐：

- default local retention；
- per-Agent/per-run override 只能更严格；
- sensitive artifact 可更短 TTL；
- cleanup 先删 payload，再修复索引；
- report 对已清理 artifact 显示 expired；
- 不在 cleanup 时改写历史 tool outcome；
- export 必须显式包含/排除 artifacts。

### 21.6 Download and UI safety

- API 下载受相同 auth；
- 验证 run/artifact ownership；
- path 不由请求直接拼接；
- `Content-Disposition: attachment`；
- filename 本地生成；
- `X-Content-Type-Options: nosniff`；
- active content 不 inline；
- range request 受 quota；
- audit download；
- UI 不把 remote HTML 当 trusted UI。

## 22. Consumer Projections

同一 envelope 生成不同投影：

### 22.1 Model projection

包含：

- status；
- bounded summary；
- selected small text；
- selected structured fields；
- artifact ID/kind/MIME/size；
- error/partial/indeterminate warning；
- provenance safe label。

排除：

- base64；
- raw headers/session；
- full huge text；
- hidden `_meta`；
- untrusted instruction elevation；
- secret fields。

### 22.2 Planner/StepRunner projection

Planner 主要看到 capability descriptor；StepRunner 在调用后看到：

- step-relevant result；
- evidence/artifact refs；
- mutation/external effect；
- validation status；
- remaining budget。

它与 [Agent Execution Lifecycle Design](2026-07-14-agent-execution-lifecycle-design.md) 的 `StepRecord` 对齐。

### 22.3 Finalizer projection

- terminal status；
- verified facts；
- evidence refs；
- failed/partial/indeterminate actions；
- artifact availability；
- 不自动拼接所有 raw content。

### 22.4 UI projection

- content block cards；
- schema validation badge；
- artifact safe preview/download；
- server/tool safe identity；
- retry/attempt；
- approval/external effect；
- 不显示 hidden secret/protocol fields。

### 22.5 Audit projection

- hashes；
- policy decisions/reasons；
- timing；
- config/capability identity；
- artifact lineage；
- redacted diagnostic；
- 不必等于用户报告。

## 23. Events and Observability

### 23.1 Canonical events

建议新增：

```text
mcp_server_connecting
mcp_server_ready
mcp_server_degraded
mcp_server_disconnected
mcp_session_initialized
mcp_session_renewed
mcp_session_closed
mcp_capabilities_change_detected
mcp_capabilities_refreshed
mcp_progress
mcp_protocol_warning
tool_artifact_created
tool_artifact_rejected
tool_call_partial
tool_call_indeterminate
```

现有通用事件继续使用：

```text
tool_call_started
tool_call_completed
tool_call_failed
approval_requested
run_cancelled
run_completed
```

MCP 事件补协议上下文，不创建第二套互相矛盾的 tool lifecycle。

### 23.2 Safe event fields

可包含：

- server config ID；
- display-safe identity；
- transport；
- protocol version；
- connection ID；
- session hash；
- capability snapshot ID；
- local capability ID；
- remote tool name（若不敏感）；
- request ID hash；
- attempt；
- duration；
- status/reason code；
- artifact ID/kind/size。

禁止：

- session raw value；
- Authorization/cookie；
- secret env；
- raw binary；
- unrestricted arguments；
-完整 server error body；
- hidden model reasoning。

### 23.3 Metrics

per server：

- connect/initialize success rate；
- request latency；
- active/pending；
- reconnect；
- session renew；
- protocol warnings；
- capability refresh；
- bytes in/out；
- artifacts accepted/rejected；
- error taxonomy；
- indeterminate count；
- retry count；
- cancellation latency。

per run：

- servers used；
- capability snapshot；
- tool calls/status；
- artifact bytes；
- degraded dependencies；
- unknown external effects。

### 23.4 Health

```text
Healthy
Degraded
Unavailable
PolicyBlocked
Misconfigured
ProtocolIncompatible
```

health 不是自动授权信号；Healthy 只表示当前协议路径可用。

## 24. State, Checkpoint, Runtime Identity, and Resume

### 24.1 Runtime identity

run identity 保存：

```text
MCP server config hashes
negotiated protocol versions
server identity hashes
capability snapshot IDs/hashes
effective tool policy version
artifact schema version
```

不保存可直接重用的 secret/session 原值。

### 24.2 Checkpoint

稳定 checkpoint 保存：

- 已 terminal 的 tool call envelope reference；
- artifact refs；
- mutations/external effects；
- indeterminate call；
- pinned capability snapshot；
- server degraded state；
- remaining budget；
- StepRecord linkage。

不把 live socket、child handle、pending channel 序列化。

### 24.3 Process restart

restart 后：

1. 重新加载 config 与 policy；
2. 比较 config/runtime identity；
3. 新建 connection；
4. 重新 initialize；
5. 获取新 capability snapshot；
6. 与 checkpoint pin 比较；
7. 已完成结果不重放；
8. indeterminate write 请求用户/验证；
9. 未发送 call 可重新准备；
10. mismatch 按兼容/拒绝/repair 规则处理。

### 24.4 Session resume vs run resume

必须分开：

- HTTP stream resume：同一 live/recoverable session 的 `Last-Event-ID`；
- MCP session renewal：404 后重新 initialize；
- rove run resume：从持久化 StepRecord/checkpoint 继续。

三者不能共用一个 `resume=true` 布尔值。

### 24.5 Capability mismatch

| 变化 | 默认行为 |
|---|---|
| server version display 变化但 schema/hash 相同 | warning，可继续 |
| tool description only 变化 | pinned run 保留旧 snapshot |
| input/output schema 变化 | 需要 rebind/replan |
| tool removed | 未执行步骤 blocked/degraded |
| safety annotation 变宽松 | 不自动放宽 |
| operator policy 更严格 | 新 policy 立即优先 |
| protocol 不再支持 | resume blocked |

runtime hard policy 永远可以比 checkpoint 更严格。

## 25. Required/Optional Servers and Degradation

### 25.1 Startup

`required=true`：

- config invalid、policy blocked、initialize/tool discovery 失败 -> runtime/profile activation 失败；
- error 必须指出 server ID 与 safe reason；
- 不能用空 registry 假装成功。

`required=false`：

- activation 可以继续；
- profile 标记 degraded；
- PlannerContext 不包含不可用 capability；
- event/UI/report 显示缺失；
- procedure selection 排除依赖该 capability 的 procedure；
- 恢复后可用于下一次 plan/revision。

### 25.2 Mid-run failure

- 当前 in-flight call 按真实 outcome；
- 后续依赖 tool 的 step blocked 或 replan；
- 不将 server outage 映射成“模型回答”；
- optional 不表示忽略失败；
- required server mid-run failure 不一定强制丢弃已获得证据；
- Finalizer 诚实报告 degraded completion。

### 25.3 Circuit breaker

可选 server 可有：

```text
Closed -> Open -> HalfOpen -> Closed
```

breaker 输入只包括明确的 transport/protocol health，不把业务 `isError` 全部算成 connection failure。状态变化发事件，且受 run/manager scope 控制。

## 26. Configuration and Interface Surfaces

### 26.1 Validation

启动前：

- unique ID；
- transport-specific required fields；
- URL/scheme/host；
- command/args；
- timeout/quota range；
- protocol version 格式；
- env reference；
- trust/session scope；
- retry policy；
- required/optional；
- egress/TLS；
- artifact policy。

无效 config 不进入连接阶段。

### 26.2 dump-config

显示：

- server IDs；
- transport；
- safe endpoint/command；
- required；
- trust；
- session scope；
- protocol support；
- timeout/quota；
- secret reference present；
- validation status。

不显示 secret/session/header raw value。

### 26.3 CLI

未来可提供：

```text
rove mcp list
rove mcp inspect <server>
rove mcp doctor <server>
rove mcp refresh <server>
```

doctor 默认：

- 只连接/initialize/list；
- 不调用业务工具；
- 产出 redacted diagnostic；
- 明确网络访问；
- 支持 JSON 输出；
- 不修改 AgentDefinition。

### 26.4 API/Web

API：

- server health；
- safe identity；
- capability snapshot；
- tool descriptor/effective safety；
- artifact metadata/download；
- degraded/indeterminate state。

Web：

- server status；
- transport/session hash；
- tool approval；
- structured result；
- artifacts；
- partial/indeterminate warning；
- refresh 后差异。

所有 mutation endpoint 需要 auth/CSRF/permission，与现有 local API policy 对齐。

## 27. Testing Strategy

### 27.1 Unit tests

- protocol version selection；
- header construction/redaction；
- server/tool identity hashing；
- alias collision；
- annotation effective policy；
- pagination loop/cursor cycle；
- JSON-RPC routing；
- retry matrix；
- status taxonomy；
- schema validation；
- block mapping；
- quota；
- URI/MIME/filename validation；
- projection/redaction。

### 27.2 Deterministic Streamable HTTP mock

本地 mock server 场景：

1. POST JSON response；
2. POST SSE response；
3. accepted/no-body；
4. initialize returns session；
5. subsequent request requires session/version header；
6. 404 session -> reinitialize；
7. DELETE 200；
8. DELETE 405；
9. GET notifications；
10. Last-Event-ID reconnect；
11. response before/after notifications；
12. concurrent out-of-order IDs；
13. malformed frame；
14. oversized response；
15. redirect/host rejection；
16. cancellation。

### 27.3 stdio regression

- existing registration/call；
- timeout；
- JSON-RPC error；
- child cleanup；
- concurrent responses；
- notifications；
- server request unsupported response；
- invalid stdout；
- stderr truncation；
- child exit with pending requests。

### 27.4 Tool discovery

- multi-page list；
- duplicate name；
- cursor loop；
- invalid schema；
- listChanged debounce；
- atomic snapshot；
- removed/changed tool；
- active run pin；
- alias collision；
- optional refresh failure。

### 27.5 Result mapping

- text；
- multiple blocks/order；
- `isError`；
- structuredContent valid/invalid/no schema；
- image；
- audio；
- resource link；
- embedded text/blob；
- unknown block；
- mixed partial；
- oversized base64；
- MIME mismatch；
- deep/large JSON；
- sensitive key redaction。

### 27.6 Retry and cancellation

- pre-send retry；
- post-commit read-only retry；
- no blind destructive retry；
- indeterminate persistence；
- cancel notification；
- disconnect not cancellation；
- timeout layer；
- late response；
- Retry-After；
- retry budget exhaustion。

### 27.7 Security

- SSRF/private IP；
- DNS rebinding simulation；
- redirect to disallowed host；
- invalid TLS；
- URL userinfo；
- secret headers not logged；
- session not persisted raw；
- path traversal filename；
- remote `file://` not local file；
- HTML/SVG active content；
- decompression bomb limit；
- artifact quota cleanup；
- untrusted annotations；
- config dump redaction。

### 27.8 Resume

- completed call not replayed；
- known-not-sent can be prepared again；
- indeterminate destructive blocked；
- process restart reinitialize；
- capability mismatch；
- policy tightened；
- missing artifact；
- expired artifact；
- server optional/required recovery。

### 27.9 Real-server smoke

保留 opt-in official filesystem stdio smoke，并增加 opt-in Streamable HTTP server smoke。real smoke：

- 不作为 deterministic correctness 的唯一证据；
- 默认关闭；
- secret/endpoint 由 env 提供；
- 输出 redacted；
- 明确 server/version；
- 不执行 destructive fixture。

## 28. Migration and Implementation Dependency Order

本文不是实现 checklist，但应按依赖顺序：

1. **Typed protocol/result foundation**

   引入 status、content block、protocol metadata、artifact ref；为现有 text ToolOutput 提供兼容投影。

2. **Shared JSON-RPC dispatcher**

   先让 stdio 使用 dispatcher，并补 concurrent/notification tests。

3. **Identity, conservative safety, pagination**

   固定 server/tool/capability identity、effective policy 和完整 catalog。

4. **Streamable HTTP adapter**

   实现 POST JSON/SSE、headers、session、GET/DELETE 与 deterministic mock。

5. **Retry/cancellation/indeterminate**

   在 commit state 与 status 已稳定后接入，不先做 blind retry。

6. **Artifact store and projections**

   按 text -> structured -> resource -> image/audio 顺序，先完成 quota/redaction。

7. **Capability refresh and run pinning**

   加 listChanged、atomic refresh、profile/lifecycle integration。

8. **Checkpoint/resume/report**

   固化 envelope/artifact/indeterminate identity。

9. **CLI/API/Web diagnostics**

   core contract 稳定后暴露。

10. **Legacy SSE deprecation**

    contract 测试和迁移完成后再评估删除。

每阶段同步：

- unit/integration tests；
- event contract；
- schema version/migration；
- current runtime docs；
- security review。

## 29. Risks and Trade-offs

### 29.1 Complexity

共享 dispatcher、session 和 artifact 会显著增加复杂度。但这是协议本身已有的复杂度；隐藏在 adapter 里只会让失败无法恢复和审计。

### 29.2 Artifact growth

保留 rich result 会增长磁盘占用。通过 quota、hash、TTL、projection 和 cleanup 控制，不能用“全部丢成文本”规避。

### 29.3 Conservative defaults reduce convenience

未知 MCP 工具默认 approval/serial 可能更慢，但比把缺失 annotation 当安全更符合 runtime 定位。operator 可通过版本化 policy 显式收紧或放宽。

### 29.4 Dynamic tool changes

run pin 会让某些实时新增工具到下一次 plan 才生效。这换来可解释与可恢复；必要时由显式 PlanRevision 采用新 snapshot。

### 29.5 No exactly-once

通用 HTTP/MCP 无法为任意远程副作用提供 exactly-once。`indeterminate` 是必须保留的诚实状态，不应被重试逻辑掩盖。

### 29.6 Protocol evolution

支持集合与 feature gates 增加维护成本，但比硬编码单一 latest 更稳定。升级必须通过 compatibility fixtures。

### 29.7 Sensitive remote content

artifact 可能含 secret/PII。默认 local、最小投影、权限下载、retention 和 redaction 是基础要求；高敏环境还需要 encryption-at-rest 的独立设计。

## 30. Acceptance Criteria

实现完成至少满足：

1. `McpTransport` 明确区分 stdio、Streamable HTTP 与 deprecated legacy SSE。
2. 三种 transport 共享 initialize、dispatcher、catalog 和 result mapping。
3. client 有明确 supported/preferred/negotiated protocol versions。
4. negotiated serverInfo/capabilities/config hash 进入 runtime identity。
5. 未 initialize 成功前不能 list/call。
6. Streamable HTTP POST 同时支持 JSON 与 SSE response。
7. 后续请求正确携带 protocol/session headers。
8. session 404 会重新 initialize，不静默复用旧 identity。
9. client 支持可选 GET stream、Last-Event-ID 和 DELETE/405。
10. session/header/secret 原值不进入普通 trace、report、dump-config。
11. dispatcher 支持并发乱序 response。
12. notifications 和 server requests 不因 ID 不匹配被丢弃。
13. unsupported server requests 得到 protocol response。
14. `tools/list` 完整分页并防 cursor loop。
15. catalog refresh 原子，失败保留上一个完整 snapshot。
16. `listChanged` 产生 safe diff，active run 保持 pinned snapshot。
17. exact remote name、本地 capability ID 与 provider alias 可追踪。
18. alias collision 不会静默覆盖工具。
19. annotations 缺失采用 conservative safety。
20. annotation 不能绕过 operator/runtime approval。
21. required/optional server 有不同、可观测的 activation 语义。
22. optional server failure 不拖垮无依赖能力。
23. retry 根据 commit point 和 effective safety 决定。
24. mutating/unknown post-commit request 不自动重放。
25. unknown outcome 映射为 `Indeterminate` 并进入 checkpoint/report。
26. disconnect 不被当作 cancellation 成功。
27. protocol cancellation、local cancellation 和 timeout 分开记录。
28. ToolOutput 能表达 Success/Partial/Error/Rejected/Cancelled/Indeterminate。
29. `isError=true` 产生 tool failure 而不是成功文本。
30. text、structured、image、audio、resource link、embedded resource 和 unknown block 有明确映射。
31. structuredContent 按 outputSchema 验证。
32. unknown/oversized/invalid content 不被静默丢弃或无限读取。
33. 大 text/base64/binary 不进入 trace/prompt。
34. artifact 有 hash、size、MIME、source、trust、validation 和 storage ref。
35. artifact 路径不使用 remote filename，remote `file://` 不映射为本地路径。
36. per-block/call/run quota 生效并清理 partial file。
37. model/UI/report/audit 使用不同安全投影。
38. API artifact download 有 auth、ownership、safe disposition 与 path boundary。
39. canonical MCP/artifact/indeterminate events 可重放。
40. restart 重新 initialize，不伪恢复旧 live session。
41. completed tool call resume 不重复执行。
42. capability/policy mismatch 有明确 resume 行为。
43. deterministic tests 覆盖 Streamable HTTP、stdio regression、retry、artifact 与攻击性输入。
44. opt-in real smoke 不替代 deterministic contract tests。
45. legacy `ToolOutput.content` 在迁移期仍有明确兼容投影。
46. AgentDefinition capability binding 与 MCP snapshot 使用同一 stable ID。
47. StepRecord/Finalizer 能引用 artifact 和 indeterminate effect。
48. `docs/runtime/` 只在实现后同步，继续作为当前事实来源。

## 31. Relationship to Existing and Future Documents

### 31.1 Existing specs

- [`2026-07-14-agent-execution-lifecycle-design.md`](2026-07-14-agent-execution-lifecycle-design.md) 消费 tool status、artifact、external effect 与 indeterminate outcome，并写入 StepRecord/Finalizer。
- [`2026-07-14-agent-definition-and-procedural-knowledge-design.md`](2026-07-14-agent-definition-and-procedural-knowledge-design.md) 定义 stable capability ID、AgentRuntimeProfile 与 procedure eligibility；本文定义 capability 如何绑定到真实 MCP server/tool。
- [`2026-05-24-rove-runtime-hardening-design.md`](../Archive/design/2026-05-24-rove-runtime-hardening-design.md) 是已归档的 state、event、tool safety、artifact 与 resume 设计背景。

### 31.2 Current-state docs

- [`docs/runtime/subsystems.md`](../runtime/subsystems.md) 继续描述当前 stdio/SSE 与 ToolOutput 行为。
- [`docs/runtime/implementation-guide.md`](../runtime/implementation-guide.md) 继续提供当前配置、启动与 MCP smoke。
- [`docs/runtime/implementation-status.md`](../runtime/implementation-status.md) 在实现和测试完成前不得因本文改成已完成。

### 31.3 Follow-up

- [`2026-07-15-oncall-reference-agent-evaluation-plan.md`](2026-07-15-oncall-reference-agent-evaluation-plan.md) 使用本篇的 mock/fixture、artifact 与 failure taxonomy 验证 Agent 机制。
- [根级 `AGENTS.md`](../../AGENTS.md) 规定维护者如何区分当前事实、未来 spec、验证证据与 secret。
- [`docs/ONBOARDING.md`](../ONBOARDING.md) 给出当前代码阅读、运行、测试和文档地图。

## 32. Design Decision

本设计的核心决定是：

> rove 的 MCP 演进不是增加一个 `streamable-http` 枚举值，而是建立一个有明确协议身份、session ownership、message dispatcher、capability snapshot、调用不确定性、结构化结果和 artifact lineage 的远程工具边界。

最终边界：

- transport 只负责消息 I/O；
- protocol core 负责 initialize、session、dispatcher 与 capabilities；
- runtime policy 决定 permission、approval、retry 和 egress；
- ToolOutput Envelope 保存真实结果类型与状态；
- Artifact store 保存大内容和二进制证据；
- projections 控制 model/UI/report/audit 各自能看到什么；
- StepRecord 固定已经发生的调用与外部效果；
- capability refresh 可以改变未来，不能改写过去；
- 无法确认的远程副作用必须被称为 `indeterminate`，不能被“自动重试成功”掩盖。
