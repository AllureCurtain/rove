# rove Runtime Hardening Design - 2026-05-24

本文定义 `rove` 下一阶段的总运行时设计方向。它覆盖当前实现和设计文档之间最重要的收口点：配置、状态、上下文、compaction、工具执行、模型提供方、memory、API/job 生命周期、CI 和交付文档。

这是一份设计 spec，不是实现计划。后续 `/goal` 应基于本文再生成可执行计划。

## Suggested /goal Objective

后续可以使用这个目标启动开发：

> Based on `docs/superpowers/specs/2026-05-24-rove-runtime-hardening-design.md`, harden rove into a local-first but remote-ready runtime: introduce layered configuration, a file-artifact plus SQLite indexed state layer, token-aware context compaction and resumable checkpoints, batch-parallel tool orchestration, a unified provider abstraction, three-layer memory with controlled promotion and recall, split CI for Rust/Web/RAG, and a documentation surface that matches the target architecture.

## Current State

`rove` 现在已经具备可运行主线，但一些关键边界仍停留在“能用”而不是“可长期演进”的阶段：

- 配置主要还是环境变量驱动。
- 状态层以 `.rove/runs/<run_id>/` 文件为主，API job 状态仍以内存 `HashMap` 为主。
- `ContextManager` 仍是固定历史长度截断。
- `Engine` 的计划路径和非计划路径存在重复，工具调用仍以单轮串行为主。
- memory 已有 `working/session/durable` 雏形，但 promotion 和 recall 还没有体系化。
- provider 层已经有 OpenAI-compatible / Anthropic / Ollama / fake，但还没有被统一成清晰的同级抽象。
- Web CI、README、配置说明和运行时对照文档还没有完全收口。

这些不是单点 bug，而是架构边界未完全定型的信号。

## Design Principles

1. **Local-first, remote-ready**
   默认只在本地运行，但内部边界要能自然演进到远程服务和多用户。

2. **Files are source artifacts, SQLite is the index**
   `.rove` 下的文件继续作为可读、可调试、可迁移的事实资产；SQLite 负责索引、恢复、列表查询、TTL 和 live 状态定位。

3. **Token-aware over message-count-aware**
   上下文管理要围绕 token budget 和稳定重建来做，不再把“保留最近 N 条消息”当成主逻辑。

4. **Structured runtime state**
   prompt、memory、tools、plan、checkpoint 都是运行时状态，而不是临时字符串拼接。

5. **Deterministic before clever**
   自动化可以很强，但 first-class fallback 必须确定、可测、可降级。

6. **Interfaces are shells**
   CLI / API / Web 都消费同一套 core/runtime 语义，不把业务状态塞回接口层。

## Target Architecture

```text
CLI / API / Web
    -> Runtime Facade
        -> Engine
            -> Context Builder + Compaction
            -> Provider Abstraction
            -> Tool Orchestration
            -> Memory Layers
            -> State Store

State Store
    -> .rove/runs/<run_id>/*  (trace / task_state / report)
    -> .rove/state.sqlite     (index / job state / replay offsets / TTL)
```

关键点：

- `Engine` 仍然是运行主循环的中心。
- `StateStore` 不再只是运行目录生成器，而是 file artifacts + SQLite 索引的组合。
- `Provider` 是 `ModelClient` 的同级实现族，不是 OpenAI-compatible 的特例树。
- `ContextBuilder` 和 `Compactor` 负责构造可恢复的 prompt state。
- `Memory` 是三层系统，不是单一 `MEMORY.md` 文件。

## Decision Summary

| Area | Target |
|---|---|
| Deployment | 本地默认，远程显式开启 |
| State | 文件 artifacts + SQLite index，优先用 `rusqlite` 封装在 `StateIndex` 内 |
| Config | `figment` typed config，优先级 `default < project config < env < CLI` |
| Context | token budget + 分段 prompt + resumable compaction |
| Tool execution | 批次并行 + 批次后顺序回写 |
| API jobs | live registry + SQLite 持久索引 |
| Provider | 统一抽象 + OpenAI-compatible / Anthropic / Ollama 同级实现 |
| Memory | working / session / durable 三层 + controlled promotion + relevance retrieval |
| Resume | checkpoint 重建优先 |
| Compaction | 自动阈值触发 + 摘要 + 重建 |
| CI | Rust/Web 分层，RAG 单独分层 |
| Docs | root README + 总设计 + 子系统设计 + 当前实现对照 |

## 1. Deployment Boundary

`rove` 的目标不是一开始就做成完整 SaaS，而是 **本地优先、远程可演进**。

### 1.1 Default Mode

- 默认只绑定 `127.0.0.1`。
- 默认不要求登录体系。
- 默认工具边界仍然以 approval policy、workspace boundary、shell validation 为主。

### 1.2 Remote-Ready Mode

系统要预留以下能力，但不强制在第一轮全部启用：

- bind address
- token auth
- CORS whitelist
- TTL / cleanup policy
- rate limiting
- future user/session isolation

这意味着：远程能力的插槽必须在架构上存在，但本地模式不能被这些能力拖慢。

## 2. Configuration System

### 2.1 Chosen Strategy

配置层采用成熟配置库，目标不是继续手写一堆 `std::env::var` 拼装逻辑。

具体选型建议：**使用 `figment` 作为配置合并层**。

原因：

- 它适合 `default / TOML / env / serialized CLI overlay` 这种多源合并。
- 可以直接反序列化到 typed config struct。
- CLI 参数可以作为最后一层 overlay 合入，而不是回写环境变量。
- 比继续扩展当前 `AppConfig::from_env()` 更容易解释、测试和文档化。

`dotenvy` 可以保留为本地开发兼容层，但它不应继续是配置系统的核心抽象。

推荐方向是 **typed layered config**，优先级固定为：

```text
default < project config < env < CLI
```

`project config` 默认是 workspace 内的 `.rove/config.toml`。

配置读取顺序固定为：

```text
AppConfig::defaults()
  -> .rove/config.toml
  -> ROVE_* env vars
  -> parsed CLI override struct
  -> AppConfig validation
```

实现上不要让 CLI 通过设置环境变量影响后续模块。CLI 应构造一个显式 override struct，并作为最高优先级配置源传给 config loader。

### 2.2 Why This Is Better

- 配置字段能保持类型安全，不再靠字符串拼接。
- 可以统一做 secret redaction。
- CLI / API / Web 可以共享同一份 config snapshot。
- 文档里的“多源配置”不再只是意图，而是实际行为。

### 2.3 Config Shape

推荐把配置拆成几个逻辑分组：

- `runtime`
- `provider`
- `tool`
- `memory`
- `state`
- `api`
- `web`
- `routing`

例如：

```rust
struct AppConfig {
    runtime: RuntimeConfig,
    provider: ProviderConfig,
    tool: ToolConfig,
    memory: MemoryConfig,
    state: StateConfig,
    api: ApiConfig,
    routing: RoutingConfig,
}
```

### 2.4 Dump and Redaction

`dump-config` 必须输出：

- effective config
- source summary
- redacted secrets
- resolved paths

但不输出：

- 原始 API key
- token
- cookie
- private MCP credential

### 2.5 Validation

配置加载完成后必须做一次 typed validation。至少覆盖：

- provider 名称是否合法
- model / fallback model 不为空
- routing failure threshold 大于 0
- cooldown / timeout 为正数
- API remote mode 开启时必须配置 token auth 或显式 unsafe flag
- workspace-relative path 解析到 workspace 内或明确允许的外部路径

validation error 应该在 CLI/API 启动阶段暴露，不能等到 engine 运行中才失败。

## 3. State, Job, and Run

### 3.1 Storage Model

状态层采用：

- `.rove/runs/<run_id>/trace.jsonl`
- `.rove/runs/<run_id>/task_state.json`
- `.rove/runs/<run_id>/report.json`
- `.rove/state.sqlite`

其中：

- 文件 artifacts 继续是可读事实资产。
- SQLite 是索引和 live-state 归档。

具体 Rust 访问层建议：**使用 `rusqlite`，并把所有数据库操作封装在 `StateIndex` / `JobStore` 这类窄接口后面**。

第一阶段不建议优先上 `sqlx`。原因是当前目标是本地嵌入式 runtime，不是远程数据库服务；`rusqlite` 更轻、更直接，也更适合 SQLite 的单机索引定位。为了避免在 async runtime 中随意阻塞，数据库 I/O 应通过以下方式之一隔离：

- 一个内部 state actor 串行处理 SQLite 请求；或
- 所有 SQLite 调用集中包在 `spawn_blocking` 边界内。

不要在 API handler、engine loop 或 tool executor 中直接散落 `rusqlite::Connection` 调用。

### 3.2 SQLite Responsibilities

SQLite 负责：

- sessions
- jobs
- runs
- event offsets
- pending approvals
- pending inputs
- TTL metadata
- last-seen / replay checkpoints

### 3.3 Live State vs Persistent State

运行中状态不应该只存在于内存 `HashMap`。

推荐拆成两层：

- `JobStore` / `StateIndex`：持久态，SQLite
- `LiveRunRegistry`：只存 active run 的 cancellation token、broadcast handle 和 task handle

这样：

- 重启后历史仍可查
- 运行中任务不会假装“自动恢复”
- live 和 persistent 的职责清楚

### 3.4 Status Semantics

为了更准确表达重启或进程丢失状态，runtime 状态建议保留并明确这些语义：

- `init`
- `running`
- `done`
- `error`
- `cancelled`
- `interrupted`

其中 `interrupted` 表示：任务没有正常完成，但也不是用户主动取消或明确错误终止，通常意味着进程重启、worker 丢失或 live registry 丢失。

### 3.5 Migration Strategy

迁移策略采用：

- **懒迁移默认启用**
- **显式导入命令可选**

也就是：

- 启动时自动创建 SQLite。
- 现有 `.rove` 文件继续可读。
- 旧文件是事实来源，SQLite 是索引视图。
- 显式迁移命令只用于补跑、验证和批量导入，不是唯一入口。

这是最适合本地优先 runtime 的方式。

### 3.6 WAL and Durability

SQLite 应启用：

- foreign keys
- WAL 模式
- bounded busy timeout
- `synchronous=NORMAL`

持久性与性能的取舍以本地优先为准，但不能把索引写成“看起来存在、实际上不可靠”的假状态。

### 3.7 Schema and Migration Discipline

SQLite schema 必须版本化。

推荐做法：

- 使用 `schema_migrations` 表记录已应用 migration。
- migration SQL 以 `include_str!` 或内部常量方式随 binary 发布。
- 启动时自动 apply 缺失 migration。
- schema 变更保持向前迁移，不要求清空 `.rove`。
- migrations 只维护 SQLite index，不迁移文件 artifacts 的事实内容。

这与懒迁移策略不冲突：旧 `.rove/runs/*` 文件仍是事实来源，SQLite 可以在首次访问、列表查询或显式导入时补建索引。

## 4. Provider Abstraction

### 4.1 Chosen Shape

provider 层采用统一抽象，`OpenAI-compatible`、`Anthropic`、`Ollama`、`Fake` 都是同级实现，而不是一个主干加几个补丁。

### 4.2 Runtime Contract

core 只看：

- `ModelClient`
- normalized `ModelEvent`
- normalized `ModelError`

core 不看 provider 原始 stream frame。

### 4.3 Normalized Stream Events

模型层必须统一输出结构化事件，例如：

- text delta
- thinking delta
- tool use start
- tool use delta
- tool use done
- usage
- done

这意味着：

- tool-use normalization 发生在 provider adapter 层
- engine 不再解析 provider-specific JSON frame

### 4.4 Routing and Fallback

Routing 层应该：

- 只在首个 committed chunk 之前切换 provider
- 一旦流已经开始产出用户可见内容，就不在半路切 provider
- 使用健康状态和错误分类决定 fallback

错误分类必须区分：

- retryable
- health-failure
- auth failure
- context length exceeded

这两个维度不能混成一类。

### 4.5 Native Providers

OpenAI-compatible 仍然是最广覆盖路径，但 Anthropic / Ollama 不能被当成兼容层的“特殊情况”，而应该作为自己的 provider 实现存在。

## 5. Context and Compaction

Claude Code 的经验说明，长会话不是“截断 history”能解决的，而是要做 **token-aware context management + resumable compaction**。

rove 需要吸收的是这个原则，不是完整照搬它的 cache 细节和交互层。

### 5.1 Prompt Structure

prompt context 的消息顺序应固定为：

```text
system
-> durable memory
-> session memory
-> compact summary
-> recent history tail
-> current user message
```

工具 schema / MCP capability 不是拼在 history 里的消息块，而是单独作为稳定 capability payload 传入模型调用。

### 5.2 Budget Rules

context 预算必须是 token-aware，而不是 message-count-aware。

至少要有三个概念：

- soft limit：达到时准备 compact
- hard limit：必须 compact 或中止
- reserved budget：给 summary / reinjection / retry 预留的空间

### 5.3 Compaction Policy

compaction 采用：

- 自动阈值触发
- 摘要生成
- 关键上下文重建
- 手动 `/compact` 能力保留

不是简单历史截断。

### 5.4 Compaction Result

compaction 产物应该是一个可恢复 checkpoint，而不是单纯一段摘要文本。它至少应包含：

- summary
- preserved tail
- plan state
- session memory pointer
- durable memory pointer
- last event / last step checkpoint

### 5.5 Resume Semantics

resume 必须是 **checkpoint 重建优先**。

这意味着：

- 先读取最新可恢复 checkpoint
- 再重建 prompt
- 完整历史只作为审计、回放和诊断来源

不是把整段旧 history 原样重放为第一优先级。

### 5.6 Compaction Failure Policy

auto compact 不能陷入无限失败循环。

推荐语义：

- 尝试摘要 / 重建
- 如果失败，做更激进的降级压缩
- 连续失败达到阈值后，停止当前 run 的自动 compact 尝试
- 如果仍无法满足上下文空间，最终以清晰的 TokenLimit / Error 终止并保留 checkpoint

也就是说：**避免无限重试，但不牺牲运行连续性**。

### 5.7 What We Do Not Adopt

rove 不在这一阶段引入 Claude Code 那类完整 prompt cache discipline、section cache boundary、side-question 重建链和复杂 UI 层 prompt 管理。

我们要的是：

- 结构化上下文
- 可恢复 checkpoint
- 自动 compact
- 失败降级

不是把整个交互系统复制过来。

## 6. Tool Orchestration

### 6.1 Execution Model

工具调用采用 **批次并行 + 批次后顺序回写**。

这比“全串行”更强，也比“全并发工具图”更稳。

### 6.2 Batch Rules

一个批次内部：

- read-only、non-destructive、互不依赖的工具可以并行
- destructive 工具必须串行并走 approval boundary
- shell / fs_write / memory write / unknown MCP destructive 默认串行

### 6.3 History Mutation Rules

批次执行期间：

- 可以并发跑工具
- 不能并发改 conversation history

批次结束后：

- 按模型给出的 tool call 顺序顺序回写
- 保持结果和 trace 的确定性

### 6.4 Hooks

hook 也要分层：

- per-tool hook
- post-run hook

未来可以预留 batch hook，但第一阶段不要把整个批次调度器和 hook 系统绑死。

### 6.5 Approval Boundary

destructive 工具的 approval 不能跳过批次语义。

如果批次中某个 destructive 工具需要批准，那么：

- 该工具暂停
- 不应越过它去执行后面的写类工具
- approval 失败后按确定性方式终止或回写拒绝结果

## 7. Memory Model

memory 采用三层：

### 7.1 Working Memory

- 当前 run 内临时状态
- 不长期保存
- 只服务于当前执行上下文

### 7.2 Session Memory

- 跨同一 session 的摘要和任务上下文
- 用于 resume 和 compact
- 存在 `.rove/memory/sessions/<session_id>.md`

### 7.3 Durable Memory

- 跨 session 的项目事实、长期偏好、稳定决策
- 仍然以文件为主：`.rove/memory/topics/*.md` + `MEMORY.md`
- SQLite 只做 metadata / index / retrieval support，不替代 Markdown 事实文件

### 7.4 Promotion Policy

promotion 不能自动把所有信息都写 durable。

只允许稳定、低风险、长期有效的内容进入 durable memory，例如：

- 项目约定
- 用户偏好
- 长期决策
- 重复出现且稳定的事实

不应自动进入 durable 的内容包括：

- secret
- 一次性输出
- 短期任务噪音
- 临时路径 / 临时调试信息

### 7.5 Relevance Retrieval

context 不应该每轮都灌完整 `MEMORY.md`。

应先做轻量 relevant recall，再按 token budget 注入结果。优先级顺序建议为：

1. session checkpoint summary
2. session memory file
3. durable memory index / topic recall
4. 当前任务上下文

relevance retrieval 不需要一开始就做成完整知识库；先做轻量、确定、可测的 recall。

## 8. API and Job Lifecycle

### 8.1 API State Shape

API 层不应继续只靠内存 `HashMap<JobId, JobRecord>`。

推荐结构：

- SQLite 保存 job / run / event / pending state
- live registry 保存 active cancellation token / broadcaster / task handle

### 8.2 Replay Model

SSE replay 不能只依赖内存 buffer，而应基于：

- SQLite event log
- seq offset
- live broadcast stream

这样 restart 后历史仍可查，active stream 也能继续 replay。

### 8.3 Restart Semantics

服务重启后：

- 历史 jobs 可查
- 历史 runs 可回放
- live task 不应隐式自动恢复执行
- 失去 worker 的任务应明确标记为 interrupted 或 equivalent terminal state

这比“看起来像还在跑”更诚实，也更适合无人值守。

### 8.4 Security Slots

API 默认本地开放，但架构要预留：

- token auth
- CORS whitelist
- rate limit
- bind address config

这些是远程-ready 插槽，不是现在就要完整 SaaS 化。

## 9. CI and Verification

CI 要分层，不要把所有重依赖都塞进单一主 workflow。

### 9.1 Recommended Split

- `ci.yml`
  - Rust default `fmt`
  - Rust default `clippy`
  - Rust default `test`
  - Web `test`
  - Web `typecheck`
  - Web `build`

- `rag-ci.yml`
  - `--features rag` clippy
  - `--features rag` tests
  - RAG index smoke

- optional `nightly-full.yml`
  - 更重的 smoke / integration / feature coverage

### 9.2 Why This Is Better

- 默认 CI 快
- RAG heavy deps 不拖慢全部提交
- Web 和 Rust 都是第一等公民
- 夜间可以跑更全的任务，不阻塞日常反馈

## 10. Documentation and Deliverables

最终交付不应该只是一堆散文式文档，而是一个有层次的体系：

1. **root README**
   - 一眼能跑起来
   - 能看懂 core / api / web / rag 的入口

2. **总架构设计**
   - 说明 runtime 主干和跨模块边界

3. **子系统设计**
   - config
   - state/job
   - context/compaction
   - provider/routing
   - memory
   - tool orchestration
   - API/security
   - CI

4. **当前实现 vs 目标设计对照表**
   - 明确现在已有、差异在哪里、为什么这样选

这比“只有一个不断堆内容的总文档”更适合长期维护。

## 11. What We Are Not Doing

以下内容不属于这轮最佳实践收口：

- 不做完整 SaaS 登录系统
- 不做分布式 job queue
- 不做 Redis 执行调度
- 不做全量 prompt cache 体系
- 不做完整知识库平台
- 不把 core 依赖接口层
- 不把所有状态都塞进 SQLite，文件 artifacts 仍然保留

## 12. Relationship To Existing Docs

这份 spec 的位置是“总运行时设计收口”，而不是替代所有已有文档。

建议的关系是：

- `docs/04-架构与路线图.md`：历史路线图，保留参考价值
- `docs/05-下一步-统一执行内核.md`：早期 deep dive，保留设计思想来源
- `docs/06-请求生命周期.md`：大而全的生命周期草案，作为参考库
- `docs/superpowers/specs/2026-05-24-rag-pipeline-hardening-design.md`：RAG 子系统专用 spec
- **本文件**：跨模块 runtime hardening 总设计

## 13. Acceptance Criteria

这份设计成立的标志是：

1. 默认运行仍然 local-first。
2. 配置有明确的多源优先级和 secret redaction。
3. 状态层采用文件 artifacts + SQLite index。
4. resume 以 checkpoint 重建为主。
5. context 以 token budget 和分段 prompt 管理。
6. compaction 可以自动触发，且有降级和熔断语义。
7. 工具调用支持批次并行，但回写顺序稳定。
8. provider 层是统一抽象，native provider 同级存在。
9. memory 是 working/session/durable 三层，并有受控 promotion。
10. API job/state 可持久索引，live handle 只保留 active 运行态。
11. CI 对 Rust / Web / RAG 分层覆盖。
12. 交付文档能从根 README 直接理解项目主线。

## 14. Design Decision

这轮设计的核心结论是：

> rove 应该成为一个 local-first、stateful、token-aware、可恢复的 runtime，而不是一个简单的 CLI 包装器或一个被接口层牵着走的 agent demo。

它应当借鉴 Claude Code 和 ragent 的关键工程思想，但只吸收那些真正能提升 rove 长期演进质量的不变量。
