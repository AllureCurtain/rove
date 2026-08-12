# rove 用户级 Provider 配置与 TUI 模型选择设计 - 2026-08-12

> Status: **Proposed / Not Implemented**
>
> Scope: 用户级配置文件、Provider catalog、CLI/TUI `/model`、API/Web
> Provider 配置收敛、运行快照与迁移。
>
> 本文不表示当前运行时已支持 `~/.rove/config.toml` 或 TUI `/model`。
> 当前事实仍以 [`docs/runtime/`](../runtime/README.md)、源代码和测试为准。
> 实现必须在审阅本文后从独立 Git worktree 开始；当前工作树只新增本设计。
>
> Implementation authority: 本文细化 Provider 配置所有权和模型选择设计，不取代
> [`Post-Full-Delivery Productization Program`](../plans/2026-08-10-post-full-delivery-productization.md)
> 的总实施权威。新 worktree 开始前，应把获批切片纳入该 program（主要是
> Provider onboarding workstream 与 TUI parity），并保持其“无 TUI 私有
> Provider/setup backend”约束。

## 1. 决策摘要

当前 Provider 的网络与协议层不需要重写。`rove-models` 已经提供规范化的
`ModelClient` / `ModelEvent`、注册式 wire protocol、受限 HTTP transport、
SSE/JSONL 解帧、认证脱敏、健康状态、重试与 fallback；OpenAI Chat、OpenAI
Responses、Anthropic、Ollama 和 external adapter 都位于这个边界内。

需要修改的是 Provider 上方的产品配置与会话装配：

1. 增加规范的用户级 `~/.rove/config.toml`，作为机器本地 Provider catalog
   和新会话默认模型的主要配置入口。
2. 项目内 `.rove/config.toml` 不再拥有 Provider endpoint、认证来源、自定义
   header 或协议选项；受信任项目最多只能选择用户 catalog 中已有的 profile
   和 model。
3. 抽取一套 CLI、API、Web 共用的 Provider catalog、模型选择和运行快照契约，
   不允许 CLI 依赖 `rove-api`。
4. CLI 不再在进程启动时永久绑定一个 `Engine`。每个新 turn 在开始前解析一次
   当前选择，构建本次运行所需的 model client / Engine，并冻结运行快照。
5. TUI 增加真正的 slash-command 路由和 `/model` picker。切换只影响后续 turn；
   活跃运行、已经落盘的运行和 resume 不会被静默换模型。
6. 普通产品启动缺少真实 Provider 时进入明确的配置引导状态，不再静默落到
   Fake。Fake 继续作为显式的测试、离线开发和 benchmark 能力。

因此，这次演进不是“重做 Provider 层”，而是保留 Provider kernel，重做其上方
的配置所有权和运行时解析边界。

## 2. 当前实现

### 2.1 已经正确的 Provider kernel

当前依赖和职责如下：

```text
apps/cli, apps/api, apps/bench
             |
             v
apps/bootstrap
  AppConfig + ProviderProfileConfig + ModelClientFactory
             |
             v
rove-models
  ModelClient / ModelEvent / routing / health
  ProviderClient / Transport / WireProtocolRegistry
  OpenAI Chat / OpenAI Responses / Anthropic / Ollama / external adapter
```

实现证据：

- [`models/src/provider/`](../../models/src/provider/) 负责 wire protocol、共享
  transport、流解码和 Provider client。
- [`apps/bootstrap/src/provider.rs`](../../apps/bootstrap/src/provider.rs) 负责
  `provider_type` 到系统 `wire_protocol` 的映射、profile 校验和密钥引用解析。
- [`apps/bootstrap/src/factory.rs`](../../apps/bootstrap/src/factory.rs) 负责把
  active/fallback profiles 装配为路由后的 `ModelClient`。
- [`docs/runtime/implementation-guide.md`](../runtime/implementation-guide.md)
  描述了当前 Provider、重试、fallback 和 health contract。

这些边界继续保留。Provider-specific payload 仍不得泄漏到 Core、Runtime、CLI、
API 或 Web。

### 2.2 当前配置路径

[`AppConfig::load`](../../apps/bootstrap/src/config.rs) 当前按下面的顺序合并：

```text
defaults < trusted workspace .rove/config.toml < environment < CLI/API overrides
```

这已经支持 TOML named profiles，例如：

```toml
[provider]
active = "team-gateway"
fallback_profiles = ["claude"]

[provider.profiles.team-gateway]
provider_type = "openai"
base_url = "https://gateway.example.test/v1"
model = "team/model"
auth = { style = "bearer", secret = { env = "TEAM_GATEWAY_KEY" } }
```

但 profile 位于 workspace 配置中；只有 Project Trust 授权后才会加载。这解决了
“不受信项目不能启动 endpoint/MCP”的问题，却没有解决“机器本地 Provider 和
认证不应由仓库拥有”的问题。当前也不存在 `~/.rove/config.toml` 用户层。

环境和 CLI override 仍是当前主要临时覆盖入口。它们适合 CI、调试和一次性
执行，不适合作为 `rove` TUI 的主要产品配置体验。

当前文档还有一项已确认的不一致：
[`docs/runtime/implementation-guide.md`](../runtime/implementation-guide.md) 的配置
章节仍有一句称 legacy flat Provider fields 保持兼容，但同一文档的 Provider
章节、当前 `AppConfig` schema 和 `ModelClientFactory` 都表明旧 flat 路径已经移除
并被拒绝。本文按可复现代码行为采用“已移除”这一事实；后续实现 worktree 必须
同步修正这句 current-state 文档，不能据此恢复旧字段。

### 2.3 当前 CLI/TUI 限制

[`apps/cli/src/cli/runtime.rs`](../../apps/cli/src/cli/runtime.rs) 在启动时解析一个
model id，构建一个 model client 和一个固定 `Engine`。TUI 后续所有 prompt
复用这个 Engine。

[`apps/cli/src/tui/reducer.rs`](../../apps/cli/src/tui/reducer.rs) 当前把 composer
中的非空文本直接变成 `TerminalAction::SubmitPrompt`。因此 `/model` 现在只是
普通用户消息，不是命令，也没有 model picker、catalog 查询、选择持久化或
并发冲突处理。

### 2.4 API/Web 已有但尚未共享的能力

API 的 ProductStore 已经拥有 Provider profile CRUD、模型 inventory、
`ProductSessionModelConfig` revision/CAS 更新，以及每次运行不可变的模型快照。
相关契约目前位于
[`apps/api/src/product/contracts.rs`](../../apps/api/src/product/contracts.rs)，
持久化实现位于
[`apps/api/src/product/store/`](../../apps/api/src/product/store/)。

这些语义值得复用，但其类型和存储目前属于 `rove-api`。CLI 不能通过依赖 API
package 来获得本地 TUI 模型选择，否则会反转现有 package 边界，并形成第二个
装配路径。

## 3. 问题定义

当前实现有四个实质问题：

1. **所有权不正确。** Provider endpoint、认证引用和 header 是机器/用户配置，
   不应以仓库文件为主要所有者。
2. **配置入口分裂。** CLI 使用 `AppConfig` profiles，Web 使用 API ProductStore
   profiles，同一台机器可能看到两套不一致的 Provider catalog。
3. **Engine 生命周期过长。** 启动时固定 Engine 使 `/model` 无法在 turn 边界
   安全切换，除非重启整个 TUI。
4. **Fake 产品默认不合适。** Fake 适合确定性测试，但真实演示启动不应假装已
   配置模型并产生 fake response。

仅在 TUI reducer 中识别 `/model` 不能解决这些问题。它会把 UI 做出来，却没有
统一 catalog、持久化、运行快照或 resume 语义。

## 4. 目标与非目标

### 4.1 目标

- 用户进入任意目录执行 `rove`，加载同一份用户 Provider catalog，并进入 TUI。
- 模型和 Provider 通过配置文件设置，不要求用户在每次启动时传 `--model` 或
  设置模型选择环境变量。
- `/model` 能发现、搜索并选择当前可用模型，结果从下一次 turn 起生效。
- CLI、API 和 Web 对 profile identity、模型选择、校验、密钥解析和运行快照使用
  同一套底层契约。
- 项目配置不能重新定义机器本地 Provider 或窃取其 credential reference。
- 活跃运行和 resume 保持可解释、可审计、不可静默漂移。
- 无真实 Provider 时明确失败或进入引导；Fake 只能显式选择。

### 4.2 非目标

- 不重写 `rove-models` 的 wire protocol 或 HTTP transport。
- 不在第一阶段实现 Provider OAuth、账号订阅或云端同步。
- 不把 API key 明文写入 TOML、SQLite、trace、report 或 UI state。
- 不允许 `/model` 热替换正在输出的模型流。
- 不让项目 `.rove/config.toml` 定义 Provider endpoint、认证、自定义 header、
  external adapter command 或任意 `wire_protocol`。
- 不删除 CI、调试所需的显式环境/CLI override；只是不再把它们作为主要 UX。
- 不在本设计文档所在的工作树直接实现代码。

## 5. 目标配置模型

### 5.1 规范位置

第一版采用一个稳定、跨平台且容易向用户解释的 Rove home：

```text
~/.rove/config.toml
```

Windows 中 `~` 解析为用户 profile 目录，因此概念和文档仍保持一致。测试、CI
和便携安装可以通过一个专用的 config-root override 指向隔离目录；该 override
只改变配置文件位置，不用于日常选择模型。具体环境变量名称在实现计划中冻结，
避免和 workspace `state_dir` 混为一谈。

用户文件负责：

- Provider profiles；
- 新终端会话的默认 profile/model/reasoning；
- fallback profiles/models；
- 用户级 Provider options；
- 只包含引用的 credential 配置。

### 5.2 建议 TOML

```toml
[model]
default_profile = "openai-main"
default_model = "gpt-example"
reasoning = "default"

[provider]
fallback_profiles = ["claude-backup"]

[provider.profiles.openai-main]
provider_type = "openai-responses"
base_url = "https://api.openai.com/v1"
default_model = "gpt-example"
auth = { style = "bearer", credential = "openai-main" }

[provider.profiles.claude-backup]
provider_type = "anthropic"
base_url = "https://api.anthropic.com"
default_model = "claude-example"
auth = { style = "header", header = "x-api-key", credential = "claude-main" }

[credentials.openai-main]
source = "keyring"
account = "default"

[credentials.claude-main]
source = "env"
name = "ANTHROPIC_API_KEY"
```

示例中的模型名是占位符，不是项目默认值。`wire_protocol` 仍由
`provider_type` 映射，用户配置不能覆盖。

### 5.3 密钥策略

配置文件形式不等于把密钥写进配置文件。目标 credential abstraction 支持：

| Source | 目标用途 | 第一阶段 |
|---|---|---|
| OS keyring | 日常交互和演示的首选 | 设计并实现后默认 |
| Environment reference | CI、临时开发、已有部署兼容 | 保留 |
| Bounded file reference | 受控本地部署 | 保留并严格校验权限/大小 |
| Literal in TOML | 不允许 | 永远拒绝 |

在 keyring 尚未实现前，第一实现切片可以先交付用户级 TOML + 现有 env/file
reference，但 UI 必须诚实显示“凭据来源”，不能称为完整的无环境变量体验。

### 5.4 权限与优先级

不能再把所有配置当作同权重的平面 merge。目标是按字段 authority 合并：

| 配置来源 | Provider 定义/认证 | 默认选择 | 项目运行策略 | 用途 |
|---|---:|---:|---:|---|
| Managed policy（未来） | 约束/允许列表 | 可约束 | 可约束 | 组织策略 |
| `~/.rove/config.toml` | 是 | 是 | 用户默认 | 日常主配置 |
| trusted `.rove/config.toml` | 否 | 仅引用已有 profile/model | 是 | 项目偏好 |
| session selection | 否 | 是 | 有限 | `/model` 与 Web 会话设置 |
| CLI/API explicit override | 不新增 durable profile | 本次启动/运行 | 本次运行 | 调试与自动化 |
| process environment | 只解析引用/兼容 override | 临时兼容 | 临时兼容 | CI/部署 |

总体优先级写成：

```text
defaults
  < user config
  < trusted project selection/policy
  < persisted session selection
  < explicit process invocation override
  < managed constraints (always enforced)
```

“优先级更高”不代表能获得更高 authority。例如项目层即使在用户层之后加载，
也不能写 Provider definition 字段；解析器必须在 merge 前按来源过滤并对被拒字段
给出 typed diagnostic，不能静默忽略。

## 6. 共享产品边界

### 6.1 共享契约

在 `apps/bootstrap` 中增加不依赖任何 UI 的产品配置模块，第一版不新建 crate：

```text
apps/bootstrap/src/user_config/
  paths.rs          # ~/.rove 与测试 override
  document.rs       # versioned TOML document
  loader.rs         # authority-aware layered load
  writer.rs         # locked + atomic update
  credentials.rs    # redacted credential references/resolution

apps/bootstrap/src/provider_catalog/
  contracts.rs      # ProviderProfileId/Profile/ModelDescriptor
  catalog.rs        # list/get/validate/inventory-facing lookup
  selection.rs      # ModelSelection + revision semantics
  resolver.rs       # selection -> immutable ResolvedRunModel
```

名称沿用既有决定：产品字段是 `provider_type`，系统字段是 `wire_protocol`；不恢复
`channel`、`OpenAI-compatible` 或旧 flat Provider 字段。

建议共享类型：

```text
ProviderProfileId       stable user-level identity
ProviderProfile         redacted, serializable profile
ModelSelection          profile_id + model + reasoning + options revision
ResolvedRunModel        endpoint identity + model + safe config digest
RunModelSnapshot        durable immutable run fact; never contains secrets
ProviderCatalogError    typed load/validation/conflict/credential errors
```

API 的 OpenAPI request/response 继续留在 `apps/api`，但转换到上述共享类型；Web
继续只通过 API 使用 catalog。CLI 直接使用共享 service，不依赖 HTTP 或 `rove-api`。

### 6.2 单一 catalog authority

用户 TOML 是 Provider definition 的可编辑 source of truth。API/Web 的 Provider
CRUD 不再维护一套独立 definition 表；它们调用共享 catalog writer，对 TOML 做
schema 校验、revision compare、文件锁、临时文件写入和同目录原子替换。

ProductStore 继续持久化：

- product session 选中了哪个 `profile_id` 和 model；
- selection revision；
- 每个 run 的 immutable model snapshot；
- profile 缺失时的显式 unresolved 状态。

ProductStore 不再复制 endpoint、credential reference 或 header 作为第二 source of
truth。读取 session 时用 stable `profile_id` 对当前 catalog 做解析；run snapshot
保留运行时使用的安全 identity/digest，以便历史解释和 resume 检查。

### 6.3 写入与并发

配置写入必须：

1. 读取当前 document revision/digest；
2. 使用 caller 提供的 expected revision 做 CAS；
3. 取得进程间文件锁；
4. 重新读取并校验，拒绝 lost update；
5. 写入同目录临时文件，flush 后原子 replace；
6. 保留未知的向前兼容字段，或在 schema version 不支持时拒绝写入；
7. 返回新的 redacted document revision。

TUI、Web 和用户手工编辑 TOML 可以同时发生。文件 watcher 只负责提示 catalog
变化；它不能修改活跃运行。解析失败时继续保留上一份已验证的内存 snapshot，
并向用户显示配置错误，不能用半解析文档覆盖有效 catalog。

## 7. `/model` 产品契约

### 7.1 命令语义

TUI composer 在提交前先经过 slash-command parser：

| 输入 | 行为 |
|---|---|
| `/model` | 打开可搜索 model picker |
| `/model current` | 显示当前 session selection 及来源 |
| `/model <query>` | 过滤 profile label、provider type 和 model id；唯一命中时选择，否则打开 picker |
| `/model reset` | 回到当前 user/project default，下一 turn 生效 |

未知 `/...` 命令显示本地 typed error，不发送给模型。需要把以 `/` 开头的文字原样
发给模型时，后续统一定义 escape 规则；第一实现切片至少不能误发一个拼错的高风险
本地命令。

Provider 创建、密钥登录和高级设置不塞进 `/model`。它们后续使用 `/provider`
或配置编辑入口，避免 picker 同时承担 catalog 管理。

### 7.2 Picker

picker 至少展示：

- profile label；
- `provider_type`；
- model id；
- credential readiness / inventory freshness；
- 当前选择和默认来源。

它支持键盘过滤、上下移动、确认和取消，使用现有 TUI overlay/reducer/effect
架构。catalog 查询和 inventory 是 effect，不在 reducer 中执行 I/O。所有集合和
字符串沿用 TUI 的 bounded/sanitized 约束。

### 7.3 状态转换

```text
TUI starts
  -> load catalog + resolve user/project default
  -> SessionSelection(revision N)

/model selection while idle
  -> validate profile/model/credential readiness
  -> persist session selection with expected revision N
  -> SessionSelection(revision N+1)

submit prompt
  -> capture SessionSelection(revision N+1)
  -> resolve model client
  -> build per-turn Engine
  -> persist RunModelSnapshot
  -> start run

/model while run active
  -> picker may inspect catalog
  -> confirmation is rejected as Busy, or queued only after an explicit
     future queue contract; it never changes the active run
```

第一版选择“active run 时禁止确认切换”，不做隐式排队。这个行为更容易解释，
也避免用户误以为正在流式输出的模型已经改变。

## 8. CLI 运行时装配

当前 `CliRuntime { engine }` 调整为长生命周期服务与短生命周期 run assembly：

```text
CliRuntime
  workspace
  base_config
  provider_catalog
  credential_resolver
  model_health_store
  state_store
  tool/environment services
  session_selection

RunAssembly (created per turn)
  resolved selection snapshot
  model client
  Engine
  approval/input providers
```

“每 turn 构建 Engine”不等于丢失会话历史。历史、memory、state、tool registry、
health store 和 execution environment 仍通过稳定 service/state identity 共享；
只把必须跟 Provider/model 绑定的对象缩短到 run 生命周期。实现前必须用现有
Engine/state contract 验证同一终端会话的上下文连续性，不能用 UI transcript
冒充模型上下文。

路由与 fallback 也在 run 开始时冻结。运行中 catalog 文件变化只影响下一次
解析，不改变已经构建的 `RoutingModelClient`。

## 9. 持久化、运行快照与 resume

### 9.1 三种不同状态

| 状态 | 可变性 | 内容 |
|---|---|---|
| Provider catalog | 用户可编辑 | endpoint、provider type、credential reference、defaults |
| Session selection | idle 时可修改 | profile id、model、reasoning、selection revision |
| Run snapshot | 不可变 | resolved provider identity digest、model、reasoning、options、catalog revision |

snapshot 绝不保存解析后的 secret。Provider identity 至少区分 profile id、
provider type、规范 endpoint、wire protocol、model 和安全配置 digest，避免两个
同名 model 共用错误的健康状态或 resume identity。

### 9.2 Resume

resume 默认保持原运行快照语义：

- 原 profile 仍存在且安全 identity 兼容时，可以重新解析 credential 并继续；
- profile 被删除、endpoint/protocol 改变或 credential 不可用时，进入 typed
  `provider_unavailable_for_resume` / `provider_changed_for_resume` 状态；
- 不允许因为当前 `/model` 选择不同而静默用新模型恢复旧运行；
- 后续如果支持“使用新模型 fork”，必须创建新 lineage/run，不能伪装成原 run
  的精确 resume。

### 9.3 Fake

保留 programmatic `AppConfig::default()` 和测试 fixture 的 deterministic Fake
能力，以满足无密钥、无网络测试不变量。产品入口使用单独的 startup policy：

- 用户显式选择 `fake`、benchmark 或测试模式：允许；
- 没有任何配置：显示 Provider onboarding；
- 配置损坏或 credential 缺失：显示具体错误；
- 不得把以上两种情况降级为 fake response。

## 10. 迁移

迁移必须是显式、可重跑、可审计的，不能在多个来源冲突时猜测。

### 10.1 来源

需要处理：

1. trusted workspace `.rove/config.toml` 中的 `[provider.profiles.*]`；
2. 现有 `ROVE_PROVIDER_PROFILES` 等 JSON/环境覆盖；
3. API ProductStore 的 Provider profile rows；
4. 当前 session model selections 和 run model snapshots。

### 10.2 规则

- 提供 dry-run，列出将导入、重命名、跳过和冲突的 profile；不显示 secret。
- 相同安全 identity 合并；同名但 endpoint/type 不同则要求用户选择新 id。
- workspace profile 迁移到用户 catalog 后，项目文件只保留允许的选择引用；工具
  不得自动改写受版本控制文件，除非用户显式确认目标文件。
- 环境 JSON 只作为一次性导入来源，不成为新的 durable authority。
- API rows 导入用户 catalog 后，ProductStore 保留 stable id mapping 和 session
  selection；确认完成前不删除旧表。
- 迁移 receipt 包含 schema version、source digest、mapping 和结果，不包含
  credential value。
- 回滚以“旧读取路径仍在一个有限版本窗口内可读”为主，不以复制明文 secret
  为代价。正式删除旧路径必须有测试和发布说明。

## 11. 安全要求

- 用户配置目录和文件创建时使用当前平台可提供的最严格合理权限。
- literal secret、URL userinfo、非法 header、CR/LF 注入、超长字段和未知 provider
  type 在网络请求前拒绝。
- `dump-config`、TUI debug、API、trace、report 和 migration receipt 只显示
  credential source metadata，不显示解析值。
- 项目配置中的 Provider definition 字段是 typed error；不能依赖“最后 merge
  覆盖掉它”来实现安全。
- 项目不能引用任意 credential id 来读取 secret；只有用户 catalog 中完整 profile
  可以解析 credential，项目只能选择 profile。
- keyring lookup、file secret 和 inventory 都要有大小、超时和错误分类边界。
- catalog watcher 不接受符号链接/替换造成的越权路径；写入时重新验证目标和父目录。
- Provider test/list-models 只针对用户明确选择的 profile，使用共享 transport 限制，
  并对错误和 header 做现有脱敏。
- active run 的 selection snapshot 一旦持久化，任何接口都不能原地修改。

## 12. 分阶段实施

### Phase 0 - 证据与冻结契约

- 为当前 AppConfig、CLI fixed Engine、API Provider CRUD 和 run snapshot 补齐
  focused characterization tests。
- 修正 current runtime guide 对 legacy flat Provider fields 的矛盾描述，并用
  schema/factory test 固定“已移除、显式拒绝”的现有行为。
- 冻结 user config schema v1、stable profile id、credential reference 和 typed
  error vocabulary。
- 明确 CLI session continuity 与 Runtime resume 所需的 durable fields。

### Phase 1 - 用户配置与共享 catalog

- 在 `apps/bootstrap` 实现 `~/.rove/config.toml` loader、authority filter、redacted
  credential resolver、atomic writer 和 catalog service。
- 先支持现有 env/file credential reference；keyring 可在同阶段完成，或明确标记
  为下一 gate，不能假称已经不依赖环境变量。
- 更新 `dump-config`，显示每个字段的来源和被拒项目字段。

### Phase 2 - API/Web 收敛

- API Provider CRUD/inventory 改用共享 catalog。
- ProductStore Provider rows 做 migration/mapping，session selection 和 run
  snapshot 保持 revision/CAS。
- Web 不改变“raw secret 不进浏览器”的边界。

### Phase 3 - CLI per-turn assembly

- 将 `CliRuntime` 拆为稳定 service 和 per-turn `RunAssembly`。
- 共享 health/state/tool/environment，验证多 turn 上下文连续性、取消和 resume。
- 普通 `rove` 缺少真实 Provider 时进入 onboarding，不再 implicit Fake。

### Phase 4 - TUI `/model`

- 实现 slash-command parser、model picker overlay、catalog effects、session
  selection persistence 和 busy/CAS error UI。
- 状态栏显示当前 profile/model，但不泄漏 endpoint credential。
- 增加 narrow-terminal、Unicode、刷新、并发编辑和 active-run negative tests。

### Phase 5 - 凭据与迁移收尾

- 完成 OS keyring、导入/dry-run/receipt、旧配置读取窗口和移除条件。
- 执行真实 Provider opt-in smoke；没有运行该 gate 时不得声明真实 Provider
  interoperability。

## 13. 验收标准

全部满足后才能把本文标记为 Implemented：

- [ ] 新用户在 `~/.rove/config.toml` 配置一个真实 Provider 后，可在任意目录执行
  `rove` 进入 TUI，并完成至少两个连续 turn。
- [ ] 日常启动不需要 `--model`、`ROVE_MODEL` 或 JSON Provider 环境变量。
- [ ] `/model` picker 能列出 catalog/model inventory，选择后只影响下一 turn。
- [ ] active run 期间切换被明确拒绝，当前流不受影响。
- [ ] CLI、API、Web 使用同一 Provider definition catalog，不存在双写 authority。
- [ ] 项目 config 尝试定义 endpoint/auth/header/adapter command 时在网络和进程
  side effect 前失败。
- [ ] run snapshot 不含 secret，且 resume 不会静默漂移到当前选择。
- [ ] 无 Provider、配置损坏和 credential 缺失均显示真实状态，不返回 Fake 文本。
- [ ] 显式 Fake、benchmark 和无网络 deterministic tests 继续通过。
- [ ] 配置写入具备 CAS、锁和原子替换测试；并发写不会丢失更新。
- [ ] workspace/env/API 旧 profile 有 dry-run、冲突、幂等和 redaction 迁移测试。
- [ ] 当前 `docs/runtime/`、README、OpenAPI 和 TUI help 在实现同一 change 中更新。
- [ ] Rust fmt/clippy/workspace tests、CLI/TUI/API focused tests、Web tests/typecheck/
  build 通过；真实 Provider smoke 单独记录执行或跳过原因。

## 14. Worktree 实施约定

本文审阅通过后再创建独立 worktree。建议从目标基线分支的最新提交创建：

```powershell
git worktree add ..\rove-provider-config -b feat/user-provider-config <reviewed-base>
```

实施前必须先处理当前工作树中尚未提交的 TUI 默认入口改动：要么先审阅并提交，
要么明确选取不包含它们的基线。不要从 dirty worktree 复制文件，也不要把当前
工作树中的用户自有 untracked evidence 带入新 worktree。

建议在新 worktree 中把 Phase 0-5 拆成单独 implementation plan 和小提交；每个
phase 都要保持 Fake deterministic path 可用，并在改变当前行为时同步更新
`docs/runtime/`。该 implementation plan 是总 productization program 的从属执行
记录，不能声明第二套产品化权威或第二个 TUI backend。

## 15. 外部产品参考

这些参考用于验证产品方向，不是 rove 必须复制的内部 contract：

- [OpenAI Codex Configuration Reference](https://developers.openai.com/codex/config-reference/)
  记录了用户级 `~/.codex/config.toml`、trusted project config，以及机器本地
  Provider/auth keys 不能被项目层覆盖的边界。
- [Claude Code settings](https://docs.anthropic.com/en/docs/claude-code/settings)
  区分 user/project/local settings scope。
- [Claude Code model configuration](https://docs.anthropic.com/en/docs/claude-code/model-config)
  将 `/model` 作为会话内模型切换入口。

rove 借鉴的是配置 scope、authority 和会话选择体验，而不是文件字段兼容性。

## 16. 仓库内参考

- [Provider Layer Redesign](./2026-07-23-provider-layer-redesign-design.md)
- [Cleanup and Naming Decisions](./2026-07-24-cleanup-and-naming-decisions.md)
- [Grok Build Reference and TUI Design](./2026-07-16-grok-build-reference-and-tui-design.md)
- [Post-Full-Delivery Productization Program](../plans/2026-08-10-post-full-delivery-productization.md)
- [Provider smoke](../runtime/provider-smoke.md)
- [Runtime subsystems](../runtime/subsystems.md)
- [Runtime implementation guide](../runtime/implementation-guide.md)
