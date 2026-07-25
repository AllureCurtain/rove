# rove Agent Definition and Procedural Knowledge Design - 2026-07-14

> Status: **Proposed / Not Implemented**
>
> 本文是未来设计 spec，不是当前实现说明，也不是实现计划。当前运行时事实仍以 [`docs/runtime/`](../runtime/README.md) 为准；在 loader、schema、事件、artifact、测试和 runtime 文档全部落地之前，不得把本文中的 Agent package、`AGENTS.md` discovery、procedure catalog 或 selection 行为描述为已实现。

> **Current-state correction (2026-07-25):** built-in vector RAG, LanceDB,
> `retrieve_code`, and `retrieve_docs` were removed from rove before the modular
> Workspace cleanup. Workspace retrieval is now bounded tools (`read_file`,
> `search_code`, `run_shell`) plus layered file memory. Any section below that
> says an existing RAG pipeline/index can be reused is a historical design-time
> assumption, not current implementation evidence. A future procedure catalog
> must start with deterministic source/metadata/lexical selection and may use a
> separately designed optional external retrieval adapter later.

本文定义 rove 中“一个 Agent 是什么”以及“Agent 如何获得可复用的做事方法”。它把 Agent 的稳定指令、工作区规则、工具能力策略、程序性知识、reference RAG、memory 和本次运行证据拆成不同类型，并规定它们的权限、版本、检索、注入、持久化与恢复边界。

本文承接 [`2026-07-14-agent-execution-lifecycle-design.md`](2026-07-14-agent-execution-lifecycle-design.md)：前一篇定义 `react` / `plan_react`、PlannerContext、bounded StepRunner、StepRecord、PlanEvaluator 和 Finalizer；本文补足其中的 `AgentRuntimeProfile`、instruction bundle、capability policy 和 procedural context 来源。

## Suggested /goal Objective

后续进入实现阶段时，可以基于本文建立独立 `/goal`：

> Based on `docs/design/2026-07-14-agent-definition-and-procedural-knowledge-design.md`, introduce versioned AgentDefinition packages and compiled AgentRuntimeProfile snapshots; define trusted instruction layering and workspace AGENTS.md discovery; add a typed, metadata-aware procedural knowledge catalog with deterministic eligibility filtering, retrieval, progressive hydration, provenance, freshness, capability binding, events, artifacts, and resume identity; keep reference RAG, memory, runtime evidence, provider configuration, and tool approval as separate authorities while preserving rove's local-first state and safety boundaries.

## 1. Scope and Evidence Boundary

### 1.1 本文解决什么

本文解决以下问题：

- Agent 的身份、版本、稳定指令、execution defaults 和 capability policy 如何被声明；
- 当前 `system.md` / `planner.md` 如何从两个松散文件演进成可验证的 Agent profile；
- 根级与嵌套 `AGENTS.md` 如何作为 workspace instructions 被发现和作用域化；
- runbook、skill、操作手册等 procedural knowledge 如何建模、验证、索引、选择和按需注入；
- procedure 如何引用能力而不是绑定某个瞬时 tool name；
- procedure、reference docs、memory、tool output 与 policy 的权限为什么不同；
- 外部文档、过期 runbook、危险命令和 prompt injection 如何被隔离；
- Agent/profile/procedure 的版本与 hash 如何进入事件、artifact、checkpoint、runtime identity 和 resume；
- 如何借鉴已归档 retrieval 设计的工程原则，而不重新引入 built-in RAG
  或把所有 Markdown 都提升成指令。

### 1.2 不存在的文件不能作为证据

用户提供的 OnCall snapshot 当前位于：

```text
../OnCall/super_biz_agent_py-release-2026-03-16
```

在本地 snapshot 中实际可见的 Markdown 是根 `README.md`、`mcp_servers/README.md` 和 `aiops-docs/*.md`。当前文件树中没有 `AGENTS.md`，也没有名称匹配 onboarding 的 Markdown。因此本文不会把未出现在 snapshot 中的文件内容归因给 OnCall；关于 Agent instructions 与 onboarding 的目标设计来自 rove 的实际缺口和通用 runtime 需求，关于 procedural knowledge 的对照证据来自 `aiops-docs`、Planner、knowledge tool 与 document splitter。

### 1.3 Source-of-truth priority

在本文尚未实现时：

1. `src/` 和测试描述当前行为；
2. [`docs/runtime/`](../runtime/README.md) 描述当前实现边界；
3. 本文描述目标设计；
4. OnCall snapshot 只作为外部机制参考，不是 rove 的规范。

### 1.4 Decision Summary

| Area | Decision |
|---|---|
| Agent identity | 用 versioned `AgentDefinition` 编译出 run-pinned `AgentRuntimeProfile` |
| Package location | 共享、可版本化的定义放在 tracked `agents/<id>/`；不把源定义混入 `.rove` runtime state |
| Legacy behavior | 当前 `prompts/system.md` + `prompts/planner.md` 映射为显式 legacy profile，避免升级即破坏行为 |
| Workspace instructions | `AGENTS.md` 是有目录作用域的 workspace policy，不是 procedure，也不默认进入 RAG |
| Procedure | procedure 是带 metadata、适用条件、能力要求、风险、验证和版本的 advisory recipe |
| Optional reference retrieval | reference/code/docs 提供事实证据，不因被检索而获得 instruction authority |
| Memory | memory 保存用户偏好、反馈、项目事实和决策，不自动保存 procedure body |
| Capability | Agent/procedure 引用稳定 capability IDs；实际 tool binding 由 runtime snapshot 完成 |
| Selection | eligibility filter 在 ranking 之前；trust、scope、platform、freshness、risk、capability 不满足时不得靠相似度入选 |
| Context | trusted policy 进入稳定 instruction prefix；procedures/memory/evidence 使用有边界的动态 context |
| Safety | definition/procedure 的 allow 或建议不能绕过 workspace、approval、destructive ordering 或 operator caps |
| Versioning | run 固定 definition/instruction/procedure content hashes；源文件中途变化不静默影响当前 run |
| Persistence | source files 是作者事实，index 可重建，run snapshots 是本次执行事实 |
| Self-modification | Agent 可以提出 procedure update candidate，但不能在同一 run 中自改并自动信任 |

## 2. Current State: rove 已有什么、缺什么

### 2.1 Prompt configuration

当前 rove 有两个 workspace-relative prompt path：

- `runtime.system_prompt_path`，默认 `prompts/system.md`；
- `runtime.planner_prompt_path`，默认 `prompts/planner.md`。

`AppConfig` 会验证这些路径默认不越出 workspace，engine assembly 将两个文件分别交给 `ContextManager` 和 `Planner`。runtime identity 已保存 system/planner prompt hash，这是很好的演进基础。

但它们目前仍只是两个文本文件：

- 没有 agent ID、definition version 或 schema version；
- 没有 capability allow/deny/require policy；
- 没有 execution/memory/knowledge defaults；
- 没有 package provenance 和 compatibility constraint；
- prompt 文件读取失败时使用内置字符串 fallback，没有 lifecycle degradation event；
- `prompts/system.md` 仍主要描述 JSON tool action，而 runtime 已同时支持 native tool-use；
- 无法区分不可覆盖的 policy 与用户可覆盖的 persona/style default。

### 2.2 Tools and capability metadata

当前 `ToolSchema` 已包含：

- name；
- description；
- JSON parameters；
- destructive；
- parallel-safe；
- 可选 `ToolCapability { status, feature, message }`。

`tool_signature()` 会按名称排序后稳定 hash schemas。工具 availability 也可通过 capability status 明确表达；当前产品不再包含 RAG enabled/stub tools。这比只在 prompt 里手写工具列表可靠。

当前缺少的是稳定的语义 capability ID、版本、risk/effect taxonomy、source identity 和 binding policy。未来 Agent/procedure 若直接写 `search_code` 或某个 MCP `query_logs` 这样的具体名称，会与 server、环境或工具重命名耦合。

### 2.3 Memory

rove 已实现 working/session/durable 三层 memory。durable topic metadata 包含：

- `type`: user / feedback / project / reference；
- `scope`: global / project / session；
- source、confidence、created/updated time。

prompt path 会按当前 user message 做 bounded lexical recall，并把 durable memory、session summary、compact summary、history 和当前请求组合进 context。`save_memory` 还会拒绝 secret signal 和明显 transient content。

这套系统适合保存偏好、反馈、项目事实、稳定决策和参考信息，但它不是 procedure catalog：

- 没有适用条件、前置检查、所需能力和风险；
- 没有步骤级 evidence/success/rollback；
- recall path 默认混合所有 memory types；
- memory promotion 的“长期有效”不等于一份操作流程已经被审核；
- Agent 不能因为一次执行成功就把整套命令自动升级为可信 runbook。

### 2.4 RAG

Built-in vector RAG has been removed. Current workspace retrieval is explicit
tool use plus layered file memory, and there is no ingestion manifest, vector
index, `retrieve_code`, or `retrieve_docs` runtime path to extend.

The missing procedure capabilities remain the same: there is no procedure
schema, trust/freshness/applicability/capability/risk filter, catalog, or
pre-planning selection service. Historical RAG design patterns such as content
hashing, staged validation, bounded lexical selection, and rebuildable indexes
may inform a future implementation, but they are not reusable current code.

“Top-k retrieved docs” must never be treated as “top-k instructions,” whether a
future retrieval backend is local or external.

### 2.5 Prompt assembly and identity

当前 prompt 的稳定顺序是：

```text
system prompt
-> durable memory
-> session memory
-> compact summary
-> recent history tail
-> current user message
```

tool schemas 单独传给 provider，prompt metadata 保存 prompt/stable-prefix/tool hashes 与 token estimate。runtime identity 还保存 workspace、model、provider、approval、plan flag、prompt hashes 和 tool signature。

目标设计应扩展这些能力，而不是另建不可观测的 prompt 拼接器。

### 2.6 Missing AgentDefinition and workspace instruction loader

当前 source tree 中没有：

- named/versioned AgentDefinition；
- Agent package discovery；
- `AGENTS.md` loader 或目录作用域解析；
- procedure metadata/schema；
- pre-planning procedure selector；
- definition/procedure lifecycle events；
- run-pinned Agent/profile/procedure artifact。

因此，当前的 rove 是“一个可配置 prompt 的 runtime”，还不是“可以安全装载多个有定义、有 procedure、有身份的 Agent profile 的 runtime”。这里的“多个”指可选择的 profile，不代表多 Agent 并行协作。

### 2.7 Current-vs-target summary

| Concern | Current | Target |
|---|---|---|
| Agent identity | system/planner prompt paths | ID + version + content hash + compiled profile |
| Instructions | 单一 system text | hard policy / operator policy / user task / defaults / advisory context 分层 |
| Workspace rules | 无 `AGENTS.md` loader | root + nested scope resolution |
| Tool declarations | tool name + basic capability status | stable capability IDs + runtime binding + Agent policy |
| Knowledge | explicit workspace tools + layered file memory | optional reference retrieval 与 procedure catalog 分权 |
| Procedure | 无 catalog/selection path | typed, validated, versioned, eligibility-filtered procedure |
| Memory | user/feedback/project/reference | 保持 memory；procedure 不自动混入 |
| Run identity | prompt hashes + tool signature | agent/profile/instruction/procedure/catalog hashes |
| Resume | 恢复 prompt/plan/memory | 恢复精确 profile 与 selected procedure snapshots |

## 3. OnCall Procedural Knowledge: 可借鉴与不可照搬

### 3.1 值得借鉴的思想

OnCall 的 `aiops-docs/*.md` 体现出 runbook 型知识相较普通 reference docs 的价值。文档通常包含：

- 告警名称、级别和触发条件；
- 问题描述；
- 顺序化排查步骤；
- 步骤目的、候选工具和参数示例；
- 常见原因与判断特征；
- 紧急、短期和长期措施；
- 验证步骤；
- 相关告警、参考文档和升级信息。

其 Planner 在制定计划前调用 `retrieve_knowledge`，把相关文档放入 `experience_context`。Markdown splitter 保留标题 metadata，knowledge tool 向模型展示来源和标题路径。这证明了三个有价值的方向：

1. procedure 应在 planning 前可用，而不是等到执行中偶然搜索；
2. 标题层级、来源和步骤结构对 procedure retrieval 很重要；
3. Planner 应知道 procedure 中建议的能力和验证方式。

### 3.2 OnCall snapshot 暴露的风险

| Observation | Risk | rove adaptation |
|---|---|---|
| runbook 没有 machine-readable frontmatter | 无法可靠过滤适用平台、版本、环境、风险和时效 | 定义 typed ProcedureMetadata，先 validate 再 index |
| 文档内硬编码地域、日志主题和工具名 | procedure 容易只适用于示例环境，却被语义检索用于其他环境 | 使用 declared parameters + stable capability IDs + applicability filters |
| diagnosis、cleanup、remediation 混在一个长文档 | 用户只问诊断时，模型可能跳到删除/清理操作 | procedure 明确 mode/risk/effects；diagnose 与 mutate 默认分离 |
| 包含 `find ... -delete`、清空日志等危险命令 | 文档相似度可能被误当作执行授权 | code fences 只是 advisory content；tool safety/approval 始终重新执行 |
| placeholder 联系方式和可能不存在的 internal links | 内容看似权威但无法验证 | required provenance、owner、reviewed/valid-until 与 link validation |
| 上传任意文档后进入同一向量库 | 外部内容被索引后没有 trust boundary | external upload 只进入 untrusted reference corpus，不能自动成为 procedure |
| top-k similarity 直接返回文档 | 相似但不适用的 runbook 仍会入 prompt | hard eligibility filtering 必须发生在 ranking 前 |
| retrieval error 以普通文本返回 | Planner 可能把“检索失败”当作经验内容 | typed success/empty/degraded/error outcome，不用文本猜状态 |
| Planner 只接收格式化字符串 | selection、hash、score、version 没进入 run identity | 返回 typed ProcedureSelection 与 source hashes |
| 没有 procedure version pin | 文档更新后无法解释旧 run 使用了什么 | run-pinned content hash + snapshot |

### 3.3 rove 应保留的优势

rove 当前保留 tool schemas、runtime identity、workspace safety、approval、
events、artifacts、layered file memory 和显式 workspace tools。Content hash、
manifest、deterministic catalog fallback 与 lexical channels 需要由未来
procedure catalog 重新建立；正确方向不是换成 OnCall 的 Milvus/LangChain
上传链路。

## 4. Design Goals

1. 定义一个可命名、可版本化、可 hash、可校验的 `AgentDefinition`。
2. 在 run 开始时将 definition、workspace instructions 和 resolved policies 编译为不可变 `AgentRuntimeProfile`。
3. 提供 legacy profile，使当前 prompt 配置可以平滑迁移。
4. 明确 policy、user task、agent defaults、procedure、memory、reference 和 evidence 的 authority。
5. 支持根级与嵌套 `AGENTS.md` 的 workspace scope，同时不把它混入 procedure RAG。
6. 定义人类可读、机器可过滤的 procedure document schema。
7. 让 procedure selection 先做 trust/applicability/capability/freshness/risk eligibility，再做相关性 ranking。
8. 借鉴已归档 RAG 设计的 ingestion、hash、channel、fallback 和 eval 原则，重新实现独立 typed corpus。
9. 让 Planner/StepRunner 通过 progressive disclosure 获取必要 procedure，而不是一次塞入所有 runbooks。
10. 用 stable capability IDs 解耦 procedure 与具体 tool/MCP server name。
11. 将 Agent/profile/procedure identity 写入 events、artifacts、checkpoint、report 和 resume diagnostics。
12. 阻止外部文档、memory 或当前 tool output 提升自身权限。
13. 允许执行反馈推动 procedure 改进，但禁止同一 run 自动自改并信任。
14. 保持 local-first、file-readable、index-rebuildable 和 deterministic-test-friendly。

## 5. Non-Goals

本文不做以下事情：

- 不实现多 Agent delegation、swarm、角色通信或 Agent-to-Agent protocol；
- 不设计在线 marketplace、自动下载远程 Agent 或自动执行安装脚本；
- 不在 Agent package 中允许任意 executable hook；
- 不让 definition 指定或携带 API key、cookie、token 等 secret；
- 不让 AgentDefinition 绕过 operator budget、workspace boundary 或 approval policy；
- 不把所有 Markdown 自动判定为 procedure；
- 不把一次成功执行自动升级为可信 procedure；
- 不用 LLM 代替 manifest validation、scope filter 或 safety enforcement；
- 不在第一阶段实现 cryptographic signature infrastructure；
- 不在本设计中重新引入已删除的 `retrieve_code` / `retrieve_docs` 或
  built-in vector RAG；未来 reference retrieval 需要独立设计；
- 不在本文中创建真实根 `AGENTS.md` 或 onboarding 内容；它们是后续交付文档；
- 不在本文中修改 Rust、配置、Web 或 runtime current-state 文档。

## 6. Knowledge and Instruction Taxonomy

### 6.1 Six distinct classes

| Class | Purpose | Typical author | Authority | Lifecycle |
|---|---|---|---|---|
| Runtime hard policy | workspace/safety/approval/budget enforcement | runtime/operator | enforced, highest | code/config version |
| AgentDefinition | stable role, policy extensions, defaults, capability/knowledge policy | Agent maintainer | trusted operator/default | package version |
| Workspace instructions | repository commands, conventions, path-specific constraints | workspace maintainer | trusted within scope | source-controlled `AGENTS.md` |
| Procedure | reusable method: when/how/validate/fallback | domain maintainer | advisory, selected | reviewed/versioned document |
| Reference + memory | facts, code/docs, preferences, feedback, decisions | docs/user/runtime | contextual evidence, not policy | indexed/recalled |
| Runtime evidence | tool output, artifacts, mutations, StepRecords | current run | factual for this run | append-only artifacts |

这六类内容不能只靠“都放进 system prompt”来区分。

### 6.2 Procedure is not memory

memory 回答“这个用户/项目过去有什么稳定事实、偏好、反馈或决定”。procedure 回答“在满足什么条件时，按什么步骤完成某类任务，并如何验证和回退”。

一条 memory 可以是：

> 本项目发布前要求运行 `cargo test --all-features`。

一份 procedure 则需要说明：

- 什么任务触发发布流程；
- 前置状态；
- 所需 capabilities；
- 每一步的目标与 evidence；
- 哪些步骤会修改外部状态；
- 成功标准、失败处理和 rollback。

### 6.3 Procedure is not a tool

tool 是可执行 capability；procedure 是使用一个或多个 capabilities 的方法。选择了 procedure 不代表 tool 一定存在，也不代表调用已被批准。

### 6.4 Procedure is not reference RAG

reference docs 可以解释 API、架构和历史背景。procedure 需要经过 schema validation、trust classification、applicability filter 和 freshness check。相似度高的 reference chunk 不能自动获得 procedure authority。

### 6.5 Runtime evidence outranks stale recipe claims

procedure 说“服务应返回 200”，而当前 health tool 返回 503 时，503 是本次事实。procedure 可以指导下一步，但不能覆盖实时 evidence。

## 7. Instruction Authority and Conflict Resolution

### 7.1 Authority layers

目标 instruction bundle 按以下 authority class 解析：

1. **Enforced runtime policy**

   由代码和 operator config 强制执行，包括 workspace boundary、approval、denylist、budget caps、remote security 和 cancellation。它不是靠模型“自觉遵守”。

2. **Trusted operator policy**

   来自显式选择的 AgentDefinition policy section 和适用的 workspace instructions。它可以进一步收紧 runtime，但不能放宽 enforced policy。

3. **Current user task and explicit constraints**

   定义本次目标、范围和交付要求。用户可以覆盖 Agent 的 style/default，但不能提升工具权限或绕过 operator policy。

4. **Agent defaults**

   persona、默认工作方式、输出风格和未被用户指定时的偏好。

5. **Advisory context**

   selected procedures、memory、reference RAG 和 tool outputs。它们提供方法或事实，不得声明自己拥有更高 authority。

### 7.2 Conflict rules

- lower authority 不能覆盖 higher authority；
- AgentDefinition 的 `allow` 不能扩张 operator allow，只能取交集；
- user 可以说“回答简短”覆盖 Agent 默认详细风格，但不能说“跳过 approval”；
- procedure 中的命令不能覆盖 workspace `AGENTS.md` 的禁止操作；
- memory 中“以前允许过”不构成本次批准；
- retrieved text 中出现“忽略之前指令”按 untrusted content 处理；
- 同 authority 的明确结构化字段优先于模糊 prose；
- 无法安全解析的冲突产生 diagnostic，并采取更保守的交集或停止，而不是随意选择最后一段文本。

### 7.3 Policy vs defaults inside AgentDefinition

Agent package 必须把不可由普通 user task 覆盖的 operator policy 与可覆盖 defaults 分开声明。不能用一个巨大 `system.md` 同时表达二者，再让 runtime 猜哪些句子是硬约束。

## 8. AgentDefinition and AgentRuntimeProfile

### 8.1 Source definition vs compiled run profile

- `AgentDefinition`：作者维护、可版本控制的 package source；
- `AgentRuntimeProfile`：runtime 在 run 开始时解析 definition、workspace instructions、capability snapshot 和 operator caps 后生成的不可变快照。

同一个 definition 在不同 workspace 或 capability 环境中可能编译出不同 profile，但每个 run 必须固定一个 profile hash。

### 8.2 Conceptual AgentDefinition

```text
AgentDefinition
  schema_version
  id
  definition_version
  display_name
  description
  runtime_compatibility
  policy_instructions_path?
  default_instructions_path
  prompt_slots
    planner?
    evaluator?
    replanner?
    finalizer?
  execution_defaults
  capability_policy
    required[]
    optional[]
    allow[]
    deny[]
  procedure_policy
    roots[]
    required_tags[]
    allowed_trust_levels[]
    max_selected
  memory_policy
    allowed_scopes[]
    allowed_types[]
    recall_limit?
    promotion_mode
  output_defaults
  owner
  tags[]
```

`definition_version` 描述作者版本；runtime 另外计算 canonical content hash。仅改换行或序列化顺序不应造成不稳定 hash，canonicalization 规则必须确定。

### 8.3 Conceptual package layout

推荐 tracked layout：

```text
agents/
  <agent-id>/
    agent.toml
    policy.md                 # optional trusted operator policy
    instructions.md           # overridable defaults and role guidance
    prompts/
      planner.md              # optional role-specific slot content
      evaluator.md
      finalizer.md
    procedures/
      <procedure>.md
    evals/
      cases.toml
    README.md                 # human docs; not injected by default
```

设计约束：

- package source 不放进 `.rove/`，因为 `.rove` 是 runtime state/artifact boundary；
- manifest 明确引用可注入文件，不能递归把整个目录塞进 prompt；
- `README.md`、evals 和示例不默认成为 instructions；
- package 内不支持自动执行 shell、Python、build script 或 install hook；
- symlink/relative path 默认不能逃出 package/workspace；
- procedure 可以来自 package 内，也可以来自 operator 明确配置的 workspace procedure roots。

### 8.4 Source selectors and shadowing

Agent selection 应使用带 source namespace 的 selector，例如：

```text
builtin:legacy
workspace:default
workspace:ops-diagnostic
future-user:research
```

同名 package 不应通过搜索顺序静默 shadow。request/config 必须解析成唯一 source + ID；未找到或冲突时返回 diagnostic。

第一阶段建议只实现 `builtin:` 与 `workspace:`。user registry、remote registry 和 signature verification 后置。

### 8.5 AgentRuntimeProfile

编译后的 profile 至少包含：

```text
AgentRuntimeProfile
  agent_selector
  agent_id
  definition_version
  definition_hash
  instruction_bundle
  instruction_bundle_hash
  prompt_slot_hashes
  resolved_execution_defaults
  resolved_capability_policy
  capability_snapshot_id
  procedure_catalog_identity
  resolved_memory_policy
  runtime_compatibility_status
  degradation_records[]
```

profile 是 engine 使用的对象，不能在每个 model turn 中重新从 mutable files 读取。

### 8.6 Provider independence

AgentDefinition 不保存 provider credential，也不默认锁死 provider/model。它可以声明能力需求，例如：

- native tool-use preferred/required；
- structured output required；
- minimum context class；
- image/input modality requirement。

具体 provider/model 仍由 runtime routing/config/request 解析。若模型不满足 required feature，profile activation 明确失败或按 policy 降级，不能静默假装支持。

### 8.7 Execution defaults cannot raise operator caps

AgentDefinition 可以建议 `plan_react`、procedure selection limit 或 step budget，但 resolved value 必须满足：

```text
resolved = request within operator policy
        or agent default bounded by operator caps
        or runtime default bounded by operator caps
```

Agent package不能通过写更大的 budget 或 `approval=auto` 扩权。

## 9. Agent Package Validation

### 9.1 Validation before activation

必须在 model/tool call 之前验证：

- manifest schema version；
- ID、version、source selector；
- runtime compatibility；
- referenced files 存在、UTF-8 可读且在允许路径内；
- instruction/prompt size 在配置上限内；
- prompt slots 不冒充 runtime-owned output schema；
- capability IDs 格式合法且 required capabilities 可解析；
- deny/allow 没有逻辑冲突；
- procedure roots 可访问且不越界；
- memory scope/type 不扩张 operator policy；
- definition 中没有 credential fields 或明显 secret material；
- package 没有 executable auto-hook；
- content hash 可稳定生成。

### 9.2 Runtime-owned prompt contracts

Planner/Evaluator/Finalizer 的 JSON schema、safe-reason requirement、budget accounting 和 tool prohibition 等 contract 由 runtime 持有。Agent package 只填充 bounded slot，例如领域术语、输出偏好和评估重点，不能替换整个 runtime contract。

这避免某个 Agent prompt 破坏 execution lifecycle 的结构化输出或安全事件语义。

### 9.3 Explicit fallback policy

- 显式请求 `workspace:foo` 且 definition invalid：默认 fail，不偷偷换成另一个 Agent；
- config default invalid：可按 operator 配置降级到 `builtin:legacy`，必须发 degradation event；
- legacy prompt file 缺失：compatibility mode 可以使用当前内置 fallback，但必须记录 source 与 fallback，不再静默；
- 不存在的 required capability：activation fail 或 blocked；不能从工具描述中猜一个等价工具。

## 10. Workspace Instructions and `AGENTS.md`

### 10.1 Role

`AGENTS.md` 表达 workspace maintainer 的长期规则，例如：

- 当前事实文档在哪里；
- 项目结构和常用验证命令；
- 哪些目录或生成文件不可手改；
- 安全、secret、migration 和 compatibility 约束；
- path-specific formatting/testing rules；
- onboarding 链接。

它不是：

- 某个具体用户任务；
- 可按相似度检索的 runbook；
- session memory；
- tool permission grant；
- 应被无限扩展的架构百科。

### 10.2 Discovery and scope

目标 discovery 语义：

1. workspace root `AGENTS.md` 作用于整个 workspace；
2. 子目录 `AGENTS.md` 作用于其目录树；
3. 对具体 target path，从 root 到最近 ancestor 依序解析；
4. deeper instructions 可以细化同 authority 的局部规则，但不能放宽 runtime hard policy；
5. 多路径操作分别解析 applicable chain；若规则冲突，使用保守交集或要求澄清；
6. source path、content hash 和 scope 必须进入 instruction bundle metadata。

第一实现阶段可以只支持 root file，但 schema/events 应保留 nested scope；不能把 root-only 偶然行为写成长期 contract。

### 10.3 Loading time

- root instructions 在 AgentRuntimeProfile 构建时加载；
- nested instruction catalog 可以在 workspace scan 时建立轻量索引；
- StepRunner/Tool boundary 在接触具体 path 前解析 applicable chain；
- 当前 run 固定已加载内容的 hash；文件中途变化只影响下一 run，除非显式 reload 并产生 profile revision；
- resume 使用 run snapshot，不静默加载最新版。

因此 profile 中的基础 `InstructionBundle` 包含 Agent instructions 与 root workspace instructions；nested files 形成 path-scoped `InstructionOverlay`，只在相关 step/tool context 中加入。不能把所有子目录规则预先求并集塞入全局 system prompt。每个首次应用的 overlay 都要 pin source hash，并与对应 target path/StepRecord 关联。

### 10.4 Instruction size and links

`AGENTS.md` 应简洁并链接到 onboarding/architecture/runbook，而不是把所有资料复制进去。loader 只注入明确文件，不自动追踪并注入所有链接；链接是给人和 Agent 按需读取的 reference。

### 10.5 Do not index as procedure by default

workspace RAG 可以将 `AGENTS.md` 作为普通 docs 供显式查询，但 procedure catalog 默认排除它。它已经通过 instruction loader 进入更高 authority；再次被 RAG 检索会重复、浪费 token，并可能产生版本歧义。

## 11. Procedure Document Model

### 11.1 ProcedureMetadata

procedure 使用 Markdown body + machine-readable frontmatter。概念 metadata：

```text
ProcedureMetadata
  schema_version
  kind = procedure
  id
  version
  status                 # draft / active / deprecated / retired
  title
  summary
  mode                   # diagnose / remediate / verify / general
  agents[]
  intents[]
  tags[]
  scope
  workspace_kinds[]
  platforms[]
  required_capabilities[]
  optional_capabilities[]
  risk_level
  side_effects[]
  declared_parameters[]
  owner
  reviewed_at
  valid_until?
  references[]
  supersedes?
  conflicts_with[]
```

`trust` 不允许由文档作者在 frontmatter 中自报。trust 由 source location、operator install/approval 和未来 signature policy 派生。

### 11.2 Example shape

以下只是 schema 示例，不代表已实现文件：

```markdown
---
schema_version: 1
kind: procedure
id: ops.disk.high-usage.diagnose
version: 1.0.0
status: active
title: Diagnose high disk usage
summary: Gather read-only disk and log evidence before proposing remediation.
mode: diagnose
agents: [ops-diagnostic]
intents: [high_disk_usage, disk_full]
tags: [ops, disk, incident]
scope: project
workspace_kinds: [folder, repo]
platforms: [linux]
required_capabilities: [system.disk.inspect, observability.logs.query]
optional_capabilities: [observability.metrics.query]
risk_level: low
side_effects: [read_only]
declared_parameters: [host, time_window]
owner: platform-ops
reviewed_at: 2026-07-01
valid_until: 2027-01-01
references: [docs/ops/disk-policy.md]
---

# When to use

# When not to use

# Preconditions and inputs

# Steps

## 1. Establish the incident window

## 2. Collect disk evidence

# Validation and success criteria

# Failure and escalation
```

### 11.3 Required body sections

active procedure 至少应表达：

- When to use；
- When not to use；
- Preconditions and declared inputs；
- Steps；
- Expected evidence / completion criteria；
- Validation；
- Failure and escalation。

如果 `side_effects` 包含 write/delete/external mutation，还必须有：

- approval expectation；
- blast-radius note；
- rollback 或明确的 non-reversible warning；
- post-change verification。

### 11.4 Diagnose, remediate, and verify should be separable

OnCall runbooks 往往把日志查询、根因判断、删除文件和验证写在同一文档。rove 默认应允许把它们分成独立 procedure，或至少通过 `mode` 与 section metadata 明确区分。

默认策略：

- 只收到“分析/诊断”请求时优先选择 `diagnose`；
- `remediate` 需要用户目标明确包含修改，且仍走 tool approval；
- `verify` 可在 mutation 之后由 Planner/Evaluator 选择；
- 不能因为 diagnosis procedure 里出现 cleanup 示例就自动执行 cleanup。

### 11.5 Capability references, not tool authorization

procedure step 应优先引用 `observability.logs.query` 之类 capability ID，而不是把 `query_logs`、server 名或 region 写死成唯一实现。具体 tool binding 由本次 capability snapshot 决定。

文档可以给参数示例，但环境相关值必须是 declared parameter 或 scoped default。示例值不应在未确认时变成真实调用参数。

### 11.6 Human-readable remains authoritative

procedure 仍以可读 Markdown 为 source artifact。index、embedding、chunk 和 selection cache 都是可重建投影。不能让向量库中的旧 chunk 反过来覆盖源文件。

## 12. Procedure Trust, Provenance, and Versioning

### 12.1 Trust levels

概念 trust level：

```text
builtin_trusted
workspace_trusted
user_installed
external_untrusted
```

- builtin 由 runtime release 提供；
- workspace trusted 来自当前 workspace 明确配置的 tracked roots；
- user installed 需要本地用户显式安装/批准；
- upload、网页、MCP resource 或临时下载默认 external untrusted。

AgentDefinition/operator policy 的 `allowed_trust_levels` 决定哪些 procedure 可进入 eligibility。trust 不是由相似度推导的单一分数；外部文档也不能通过在 frontmatter 写 `trust: builtin` 来升级自己。

### 12.2 Provenance

每个 catalog entry 至少记录：

- source selector/root；
- relative path；
- package/agent association；
- declared version；
- canonical content hash；
- optional git commit/worktree fingerprint；
- owner/review time/expiry；
- validation status；
- index generation/version。

### 12.3 Version selection

- 同一 procedure ID 可以存在多个版本，但一次 run 只选择一个确定版本/hash；
- 默认选择 eligibility 范围内未 deprecated 的最高兼容版本；
- 显式 pin 优先，但仍需通过 trust/capability/safety validation；
- retired 版本不用于新 run；
- resume 必须使用原 hash snapshot，不能自动切到 latest；
- `supersedes` 只描述 lineage，不删除旧 run evidence。

### 12.4 Freshness

- `valid_until` 已过且 procedure 标记 hard-expiry：从自动选择中排除；
- soft-expiry 可以入候选但必须降权并显示 warning；
- 没有 freshness metadata 的 workspace procedure 可按 operator policy允许，但不能假装已审核；
- capability/API/version-sensitive procedure 应要求 review window；
- freshness 是 eligibility/diagnostic，不由向量相似度抵消。

## 13. Procedure Catalog and Indexing

### 13.1 Reuse design principles, not removed infrastructure

当前仓库没有可复用的 built-in RAG pipeline。procedure catalog 可以借鉴已归档设计中的：

- staged scan/parse/chunk/embed/persist；
- content/chunk hash；
- manifest 与 append-only stage log；
- vector/lexical/path channel；
- dedupe、normalization、rerank boundary；
- deterministic embedder、manifest fallback 与 eval。

但必须重新实现 typed procedure semantics。若未来引入通用 retrieval
components，应由 optional reference retrieval 和 procedure catalog 分别使用；
core engine 只依赖窄的 `ProceduralKnowledgeProvider` contract，不直接依赖某个
vector database。

procedure metadata validation、source catalog 和 deterministic selection 属于
Agent runtime baseline，默认 build 必须能从 validated source/manifest 做
metadata + lexical selection。Vector/model ranking 只能作为后来单独设计、可
关闭的增强，不得成为 baseline 正确性的前提。

### 13.2 Separate logical corpus

procedure 不应只是现有 `RetrieveKind::Docs` 的一个 tag。目标需要独立 logical corpus，例如：

```text
KnowledgeCorpus
  code
  reference_docs
  procedures
```

物理上可以复用 storage adapter，但 manifest、filters 和 evaluation 必须能按 corpus 隔离。建议的 state artifact namespace 是：

```text
.rove/knowledge/procedures/manifest.json
.rove/knowledge/procedures/index_log.jsonl
.rove/knowledge/procedures/<index backend artifacts>
```

最终路径可在实现计划中调整；不变量是 source 与 index 分离、corpus authority 分离、index 可重建。

### 13.3 Validation precedes indexing

ingestion stages 应为：

```text
Discover configured roots
-> Parse frontmatter and body
-> Validate schema/paths/capabilities/links/risk sections
-> Derive trust and provenance
-> Build catalog entries
-> Structure-aware procedure chunking
-> Embed/index eligible searchable content
-> Write manifest and diagnostics
```

invalid procedure：

- 不进入 active selection index；
- 出现在 validation report；
- 保留 source file，不由 indexer自动修改；
- 不因为 body 可正常分块就忽略 metadata 错误。

### 13.4 Procedure-aware chunking

普通 Markdown chunking 可能把“执行命令”与前面的 precondition/后面的 rollback 分开。procedure chunker 至少应保留：

- frontmatter identity；
- heading path；
- step boundary；
- risk/side-effect metadata；
- procedure ID/version/hash；
- section type；
- prerequisite and validation references。

retrieval 命中某个 step chunk 后，hydration 应能重新加载必要的 preconditions、step、validation 和 failure sections，而不是只把孤立命令片段交给模型。

## 14. Procedure Selection Pipeline

### 14.1 Selection is a runtime service

procedure selection 由 injected `ProceduralKnowledgeProvider` 在 PlannerContext 构建前执行。Planner 不通过普通 tool 自由搜索并决定哪些文档获得 instruction status。

这保持：

- `core` 不依赖具体 vector store；
- selection status 是 typed result；
- PlannerContext 可以记录 catalog/hash/score/provenance；
- reference material 仍通过当前显式 workspace tools 获取；未来 optional
  retrieval tool 不会因返回文档而授予 instruction authority。

### 14.2 Selection input

```text
ProcedureQuery
  original_goal
  user_constraints
  agent_id / profile hash
  execution_strategy
  workspace kind / platform
  target path hints
  capability snapshot
  allowed risk/effect policy
  language/locale hints
  prior StepRecords?          # replan/selection revision 时
```

query 构建不能包含 secret、raw credential 或不必要的完整 tool output。

### 14.3 Hard eligibility before ranking

候选必须先通过：

1. schema/validation status；
2. active/deprecated policy；
3. trust level 被 Agent/operator policy 允许；
4. agent applicability；
5. workspace scope/kind；
6. platform/runtime compatibility；
7. required capabilities availability；
8. operator allow/deny；
9. risk/effect compatibility；
10. freshness/expiry；
11. explicit conflicts。

任何 ranking 分数都不能让不合格候选重新入选。

### 14.4 Ranking

eligibility 后可组合：

- intent/tag exact match；
- lexical match；
- vector semantic relevance；
- title/summary/heading match；
- capability completeness；
- target path/scope match；
- user explicit pin；
- reviewed freshness；
- optional historical evaluation signal。

ranking 输出必须保留 score breakdown，不能只给一个无法解释的总分。权重通过 evaluation 决定，不照搬 OnCall `top_k=3` 或任意 prompt heuristic。

### 14.5 Deduplication and threshold

- 按 procedure ID/version/hash 去重；
- 同 ID 多版本按 version policy 选一个；
- 低于 relevance threshold 返回 no-match，不为了凑 `max_selected` 注入无关 procedure；
- `max_selected` 是上限，不是必须数量；
- mutually conflicting procedures 不能同时注入，除非 Planner 明确需要比较并且 context 标记冲突；
- user pin 也不能绕过 hard eligibility。

### 14.6 Typed selection result

```text
ProcedureSelection
  selection_id
  query_hash
  catalog_hash
  status                    # selected / empty / degraded / error
  matches[]
    procedure_id
    version
    content_hash
    trust
    score_breakdown
    matched_intents/tags
    capability_bindings
    freshness_status
    selected_sections
  rejected_summary[]        # safe reason codes, bounded
  fallback_used?
```

typed `empty` 与 `error` 必须不同，不能像普通文本一样让 Planner 自行判断“没有找到”或“检索失败”。

### 14.7 Deterministic baseline

第一阶段以 metadata filter + lexical/tag/path ranking 为可测试 baseline。vector 和 model rerank 是增强路径；index/backend 不可用时，仍可从 validated manifest/source catalog 做 deterministic selection。

## 15. Progressive Disclosure and Hydration

### 15.1 Why not inject full runbooks

长 procedure 可能包含 diagnosis、多个原因分支、remediation、verification 和 references。一次性注入多个完整 runbook 会：

- 浪费 context；
- 混淆当前 step；
- 放大过期/危险命令的影响；
- 降低 stable prompt cache；
- 使 Planner 难以区分核心 recipe 与细节。

### 15.2 Disclosure levels

推荐四级：

1. **Catalog summary**

   Planner 看到 ID/version/title/summary/mode/risk/capability/why-selected。

2. **Planning outline**

   对最终选中 procedure，Planner 看到适用条件、preconditions、step headings、success/failure outline。

3. **Step hydration**

   StepRunner 只加载当前相关 step、必要 preconditions、evidence criteria、validation/rollback。

4. **Audit/finalization references**

   Evaluator/Finalizer 通常只需要 procedure references、adherence/deviation 和 StepRecords，不重新注入全文。

### 15.3 Hydration invariants

- hydrated sections 必须来自已 pin 的 content hash；
- 不能从更新后的 source file 混入相同 ID 的新内容；
- section dependency 一并加载，例如 remediation step 不能没有 rollback note；
- token limit 优先删减 examples/reference prose，不截断 runtime policy、preconditions 或 safety section；
- hydration event 记录 section IDs/hashes，不记录可能含敏感内容的全文。

## 16. Capability Contracts and Tool Binding

### 16.1 Stable capability identity

当前 `ToolCapability` 主要表达 enabled/feature/message。目标 capability descriptor 至少需要：

```text
CapabilityDescriptor
  id
  version
  status
  source                   # builtin / mcp / future extension
  tool_name
  input_schema_hash
  effect_class
  risk_class
  approval_requirement
  parallel_safe
  availability_reason?
```

最终字段与 MCP artifact/transport 由后续专门 spec 收口，但 `id`、availability、effect/risk 和 binding 是本文需要的不变量。

### 16.2 Binding

```text
procedure required capability
-> Agent capability policy
-> runtime capability snapshot
-> one eligible concrete tool binding
-> normal ToolSchema validation and approval
```

binding 结果进入 ProcedureSelection 与 PlannerContext。若多个 tool 提供同 capability，runtime 可按 operator policy、source trust 和 availability 选择；模型不应仅凭名字碰运气。

### 16.3 Capability policy is restrictive

- `required` 表示 Agent 激活所需；
- `optional` 影响可用 workflow，不导致自动失败；
- `allow` 是 Agent 可见/可用集合的进一步限制；
- `deny` 优先于 Agent allow；
- operator/runtime deny 始终优先；
- capability available 不代表 destructive action 已批准；
- procedure 的 capability requirement 不增加 capability。

### 16.4 MCP compatibility

没有稳定 semantic capability metadata 的 MCP tool 可以临时映射为 namespaced `tool:<server>/<name>`，但这种绑定应标记 unstable。高风险或 required procedure 不应依赖模糊的 name-only 自动等价。完整 MCP capability/resource/artifact metadata 在后续 spec 中设计。

## 17. Integration with Agent Execution Lifecycle

### 17.1 Run startup order

在前一篇 lifecycle 的基础上，`plan_react` 正常顺序扩展为：

```text
allocate run identity
-> resolve and validate AgentDefinition
-> build AgentRuntimeProfile + workspace instruction bundle
-> build capability snapshot
-> select execution strategy
-> select procedures
-> build PlannerContext
-> create plan
-> bounded StepRunner / Evaluator / Finalizer
```

事件层可以在 `run_started` 后依次发 profile、strategy 和 procedure selection events；任何失败都必须形成明确 terminal/degradation state。

### 17.2 Planner integration

PlannerContext 获得：

- AgentRuntimeProfile identity；
- trusted instruction bundle；
- selected procedure catalog summaries/outlines；
- capability bindings；
- memory 与 reference context；
- user/runtime constraints 和 budgets。

PlanDraft step 可以引用：

```text
procedure_ref = <id>@<version>#<section>
capability_ref = <stable capability id>
```

引用帮助 provenance，不把 procedure 变成必须逐字执行的脚本。

### 17.3 StepRunner integration

StepRunner 在当前 step 开始时 hydrate 必要 sections，并记录：

- applied procedure refs；
- resolved capability bindings；
- required preconditions；
- expected evidence/completion hints；
- risk/effect notes。

它仍通过 bounded ReAct 决定具体 tool calls。procedure 不是宏执行器。

### 17.4 Deviation

允许因实时 evidence、capability failure 或用户新约束偏离 procedure，但必须：

- 不违反 higher authority policy；
- 记录 `procedure_deviation` safe reason code；
- 在 StepRecord 中引用原 procedure 与 deviation；
- 重大偏离可触发 PlanEvaluator `replace_remaining`；
- 不能用“偏离 procedure”为绕过 approval 的理由。

### 17.5 Selection revision

如果执行中发现原 intent 错误或需要不同 procedure：

- 新 selection 只能由 explicit lifecycle transition 触发；
- 保存 parent selection ID、触发 StepRecord 和新 catalog/query hash；
- 已执行 StepRecords 不改写；
- 新 procedure 重新做 eligibility；
- 受 plan revision、model/tool/token/time budgets 限制。

### 17.6 Finalizer

Finalizer 接收 procedure IDs/versions/hashes、哪些 sections 被应用、重要 deviation 和对应 StepRecords。它不需要再次读取 procedure 全文，也不能把“遵循了 procedure”当作成功证明；成功仍由实际 evidence 和 validation 决定。

### 17.7 React strategy

`react` 不经过 Planner，但可以：

- 在用户显式 pin procedure 时加载；
- 在 deterministic selector 高置信匹配且 Agent policy 允许时注入 bounded advisory outline；
- 完全 no-match 时保持当前直接 ReAct，不强制增加检索成本。

procedure injection 是否值得用于短任务应由 evaluation 决定。

## 18. Memory, Optional Reference Retrieval, and Procedure Feedback

### 18.1 Keep stores logically separate

| Store | May contain | Must not imply |
|---|---|---|
| Durable memory | user preference, feedback, project fact/decision, reference note | verified procedure or permission |
| Session memory | current work summary and resume facts | cross-project policy |
| Optional reference retrieval | code/docs/API/reference material | trusted instruction authority |
| Procedure catalog | reviewed recipes with metadata | automatic tool authorization |
| Runtime evidence | current tool results/artifacts/mutations | universal future rule |

### 18.2 Memory can influence selection, not trust

memory 可以提供“本项目使用 Linux”“用户偏好只做诊断不自动修改”等上下文，用于 eligibility/query；它不能把 external procedure 提升为 trusted，也不能覆盖 manifest platform/risk。

### 18.3 Feedback loop

执行完成后可以记录：

- procedure selected/not selected；
- preconditions 是否满足；
- applied/deviated sections；
- tool/capability binding success；
- StepRecord outcome；
- user feedback；
- stale/missing step signals。

这些反馈进入 evaluation/report 或受控 feedback store，不自动编辑 source procedure。

### 18.4 Candidate promotion

Agent 可以生成 procedure improvement candidate，例如：

- 建议新增适用条件；
- 修正过期参数；
- 补充 failure branch；
- 将成功的新路径整理成 draft。

candidate 必须：

- 状态为 draft/untrusted；
- 带 source run/StepRecord/evidence refs；
- 通过 validation 和人工/显式 operator review；
- 在后续 run 才能成为 active trusted procedure。

禁止 “写完 procedure -> 当前 run 立即 reload -> 按新 procedure 扩权”。

## 19. Context and Prompt Assembly

### 19.1 Target sections

目标 prompt/context 逻辑顺序：

```text
stable instruction prefix
  runtime contract and enforced-policy summary
  AgentDefinition operator policy
  Agent defaults
  applicable workspace instructions

dynamic advisory/context blocks
  selected procedure summaries or hydrated sections
  durable/session memory
  compact summary and recent history
  reference evidence when explicitly retrieved
  current user message

tool capability payload
  separately passed schemas and capability snapshot
```

实际 provider role 映射可以不同，但 authority 与边界必须保留。

### 19.2 Procedures should not be concatenated into policy text

selected procedure body 使用明确 delimiter、source ID/version/hash 和 advisory label，放在动态 context block。不能把检索到的原始 Markdown直接拼进 Agent policy section，否则 external/prompt-injected content 会获得错误的 system authority。

### 19.3 Token priority

token 压力下的保留优先级：

1. runtime contract 与 enforced policy summary；
2. active Agent/workspace policy；
3. current user goal/constraints；
4. 当前 step 必需 procedure preconditions/safety/validation；
5. capability schemas；
6. relevant StepRecords/evidence；
7. memory；
8. procedure examples/reference prose；
9. older history tail。

不能先截断 safety/preconditions，留下孤立命令示例。

### 19.4 Prompt metadata

目标 `PromptBuildMetadata` 可扩展：

- agent profile hash；
- instruction bundle hash；
- workspace instruction hashes/scopes；
- selected/hydrated procedure hashes；
- procedure selection ID；
- capability snapshot ID；
- memory/reference context hashes；
- section-level token counts；
- stable/dynamic prefix hashes。

这有助于 prompt cache、debug 和 resume，但 metadata 不应包含 secret 或全文。

### 19.5 Injection resistance

- frontmatter 与 body 分开解析；
- external reference content 始终标记 untrusted；
- procedure body 不能声明自己的 trust/authority；
- “ignore previous instructions” 等内容不改变 authority；
- HTML/Markdown hidden content、oversized comments 和 encoded payload 应在 ingestion/validation 中诊断；
- runtime safety靠 tool boundary 执行，不依赖 prompt injection detector 的完美准确率；
- procedure commands 只作为文本，必须转成正常 typed tool call 才可能执行。

## 20. Persistence, Artifacts, Runtime Identity, and Resume

### 20.1 Source, index, run snapshot

三类事实分别是：

- **Source definition/procedure files**：作者维护的当前版本；
- **Catalog/index artifacts**：从 sources 可重建的检索投影；
- **Run snapshots**：某次 run 实际使用的 profile、instructions 和 procedure content identity。

不能用 index 代替 source，也不能用最新版 source 解释旧 run。

### 20.2 Suggested run artifacts

在现有 `trace.jsonl`、`task_state.json`、`report.json` 旁，目标可以增加：

```text
.rove/runs/<run_id>/agent_profile.json
.rove/runs/<run_id>/procedure_selection.json
.rove/runs/<run_id>/procedure_snapshots/<id>@<hash>.md
```

字段/路径可在实现计划中优化，但必须满足：

- exact content identity 可审计和 resume；
- selected procedure 数量/体积受限；
- snapshot 不包含 secret；
- artifacts 保持本地可读；
- SQLite 只做索引，不成为唯一 source。

### 20.3 TaskState and checkpoint

目标状态至少保存：

```text
agent_selector
agent_definition_version/hash
agent_profile_hash
instruction_bundle_hash
workspace_instruction_refs/hashes
procedure_catalog_hash
active procedure selection IDs
selected procedure version/content hashes
hydrated section high-water marks
selection revisions
capability snapshot/binding hashes
degradation records
```

完整文本可以通过 run artifact pointer 获取，不必全部内嵌 checkpoint。

### 20.4 RuntimeIdentity

在前一篇 execution lifecycle 建议的 strategy/policy/budget hashes 之外，runtime identity 应加入：

- agent source selector；
- definition version/hash；
- instruction bundle hash；
- workspace instruction scope/hash set；
- procedure catalog generation/hash；
- selected procedure hashes；
- capability policy/binding hash；
- memory/procedure policy hash。

当前 engine assembly 可以在 user goal 出现前计算静态 runtime identity，但 selected procedures 是 goal-dependent。目标实现不应让静态 identity 假装预知动态 selection。可以明确拆成：

- `BaseRuntimeIdentity`：workspace、provider/model、Agent profile、instruction bundle、capability/catalog generation 和 policy hashes；
- `RunContextIdentity`：execution strategy、selection ID、selected procedure hashes、bindings 和 resolved budgets。

如果最终仍使用一个序列化结构，也必须在 procedure selection 后完成 run-specific identity projection，并把它写入 task state/checkpoint/report；不能只保留 engine construction 时的旧快照。

### 20.5 Mid-run source changes

- source definition/procedure 改动不影响当前 immutable profile；
- file watcher 可以提示 `source_changed`，但不自动 reload；
- 显式 reload 创建 profile/selection revision，并需要 lifecycle 支持；第一阶段可直接要求新 run；
- Agent 修改自身 package 后，当前 run 仍使用旧 snapshot；
- tool registry 变化走 capability mismatch，不偷偷重新绑定。

### 20.6 Resume

resume 顺序：

1. 读取 run-pinned agent profile/selection snapshots；
2. 校验 artifact hash 与 task state/runtime identity；
3. 当前 runtime hard policy 始终重新生效；旧 snapshot 不能绕过新安全修复；
4. 若 source 最新版不同但 snapshot 完整，默认继续旧 semantic profile并记录 mismatch；
5. 若 snapshot 缺失，只能使用 exact-hash source；
6. exact content 不可得时要求 restart/confirm，不能加载 latest 假装一致；
7. capability binding 不再可用时触发 re-evaluation/degradation；
8. remaining budget 和 procedure selection revision 不重置。

## 21. Events and Observability

### 21.1 Target events

| Event | Purpose |
|---|---|
| `agent_profile_resolved` | selector、definition version/hash、profile hash、compatibility status |
| `agent_profile_degraded` | legacy fallback、missing optional slot、capability/model degradation |
| `instruction_bundle_built` | source hashes、workspace scopes、token counts，不含全文 |
| `instruction_overlay_applied` | target path、nested `AGENTS.md` scope/hash 与关联 step/tool boundary |
| `procedure_catalog_ready` | catalog generation/hash、valid/invalid/stale counts、fallback backend |
| `procedure_selection_completed` | selected/empty/degraded/error、selection ID、safe score/rejection summary |
| `procedure_hydrated` | procedure/section hashes 与 step association |
| `procedure_deviation` | StepRecord、safe reason code、是否触发 plan/selection revision |
| `procedure_selection_revised` | parent selection、trigger evidence、新 selection identity |

如果 definition 在 run allocation 前已解析，runtime 仍应把 resolved identity作为 run 的首批事件/artifact记录；接口层不能只在启动日志里打印后丢失。

### 21.2 Ordering with execution lifecycle

正常 `plan_react` 事件顺序扩展为：

```text
run_started
-> agent_profile_resolved
-> instruction_bundle_built
-> execution_strategy_selected
-> procedure_selection_completed
-> plan_created
-> plan_step_started
-> procedure_hydrated
-> model/tool events
-> step_result
-> plan_decision / optional selection or plan revision
-> finalization
-> run_completed
```

### 21.3 Safe event payloads

事件可包含 ID、version、hash、path scope、status、score breakdown、reason code、token count 和 capability IDs。默认不包含：

- 完整 policy/instructions；
- procedure 全文或危险 command text；
- secret-bearing parameters；
- hidden reasoning；
- private external document content。

### 21.4 Metrics

建议投影：

- Agent selector/profile 使用分布与 fallback rate；
- invalid definition/procedure 数量；
- procedure selection precision/empty/error/degraded rate；
- eligibility rejection by trust/platform/capability/freshness/risk；
- selected procedure 数量和 context token overhead；
- procedure adherence/deviation/outcome；
- stale procedure hit rate；
- capability binding failure；
- resume profile/procedure mismatch；
- external-untrusted content 被错误提升为 procedure 的数量，目标为零。

## 22. Security and Trust Boundaries

### 22.1 No permission by prose

AgentDefinition、`AGENTS.md` 或 procedure 中出现 “auto approve”“可以删除” 不会改变 runtime `ApprovalPolicy`。权限必须来自 operator config 和 interface approval channel。

### 22.2 External content stays untrusted

upload、URL、MCP resource、email/chat attachment 或 tool output 默认只能进入 reference/evidence boundary。升级为 procedure 必须显式 import、校验、review 和 trust assignment。

### 22.3 Path and symlink safety

- definition/procedure roots 默认在 workspace 或批准 registry 内；
- canonical path validation 防止 `..`、junction/symlink escape；
- manifest 引用不能读取任意本机 secret file；
- `state.allow_external_paths` 不应自动等价为 “external agent package trusted”；两者需要独立 policy。

### 22.4 Secret handling

- manifest 不定义 secret value fields；
- prompt/procedure validation 检测明显 secret signal；
- config dump 和 events 只显示 selector/path/hash；
- tool/MCP credentials 由现有 config/transport 管理；
- procedure 可引用 credential capability requirement，但不能包含 credential 本身。

### 22.5 Dangerous snippets

- code fence 永远只是 advisory；
- destructive command 必须在 metadata/section 中标记 risk/effect；
- 未声明 side effect 的危险 snippet 导致 validation warning/error；
- 执行时重新通过 shell validation、workspace boundary 和 approval；
- diagnosis selection 不自动 hydrate remediation command sections。

### 22.6 Self-modification and TOCTOU

- run 使用 content-addressed snapshots；
- current Agent 可以修改 source file，但不能改变已编译 profile；
- index rebuild 与 run selection 使用 generation ID；
- selection 后 source/index 变化不替换已 pin content；
- 新内容进入 active trusted catalog 必须经过下一次 validation/activation boundary。

### 22.7 Trust is not a complete security sandbox

`workspace_trusted` 只表示 operator 允许它作为 procedure source，不代表内容绝对正确。tool safety、approval、evidence validation 和 budget 仍然必须存在。

## 23. Failure and Degradation Semantics

| Failure | Target behavior |
|---|---|
| Explicit Agent selector not found | fail validation；不换成同名其他 source |
| Config-default Agent invalid | 按显式 fallback policy fail 或降级 legacy，并发 event |
| Prompt/instruction file missing | explicit package invalid；legacy fallback 需记录 degradation |
| Runtime incompatibility | fail 或使用 manifest 允许的 compatibility mode，不静默 |
| Required capability missing | profile activation blocked，或 Agent 明确声明的 degraded mode |
| Optional capability missing | profile degraded；procedure eligibility 同步更新 |
| `AGENTS.md` unreadable | root policy 默认 fail/require acknowledgement；optional nested scope 可按 operator policy degrade |
| Workspace instruction conflict | conservative intersection + diagnostic；无法安全解析时停止相关 path action |
| Procedure parse/schema error | 排除 active index，写 validation report |
| Broken reference/link | warning 或 error 取决于是否属于 required precondition/safety source |
| Procedure hard-expired | 自动选择排除；显式 pin 需要 operator acknowledgement |
| Procedure index unavailable/corrupt | 从 validated manifest/source catalog deterministic fallback；否则 typed degraded/error |
| No eligible procedure | typed empty；optional selection 下继续，不注入无关 docs |
| Required procedure missing | blocked/fail，不用 reference doc 冒充 |
| Selection timeout | typed degraded；按 Agent policy no-procedure continue 或 fail |
| Hydration hash mismatch | 拒绝内容，重新从 pinned snapshot读取或 fail |
| Procedure exceeds token budget | 保留 preconditions/safety/current step/validation，裁剪 examples；仍超限则不使用并报告 |
| Capability binding changed mid-run | selection/plan re-evaluation；不静默换高风险工具 |
| External prompt injection | 保持 untrusted boundary；若 parser/validator无法安全处理则排除 |
| Run snapshot missing on resume | 只接受 exact-hash source；否则 require restart/confirmation |
| Agent writes new procedure in current run | 保持 draft candidate；不进入当前 active catalog |

所有 degradation 必须：

- 有 typed status；
- 进入 trace/report/runtime identity；
- 不扩张权限；
- 不把 reference/memory 自动升级为 procedure；
- 不把 latest source 冒充旧 snapshot；
- 不因检索失败制造假 runbook。

## 24. Configuration and Interface Surface

### 24.1 Conceptual config

最终字段命名由实现计划校准，语义可接近：

```toml
[runtime.agent]
selector = "builtin:legacy"
fallback_policy = "fail_explicit_degrade_default"
workspace_instructions = true
nested_workspace_instructions = false

[knowledge.procedures]
roots = []
allowed_trust_levels = ["builtin_trusted", "workspace_trusted"]
max_selected = 3
selection_mode = "deterministic"
allow_soft_expired = false
snapshot_selected = true

[knowledge.procedures.index]
enabled = true
backend = "manifest"
```

Agent package defaults 与这些值合并后，operator cap/deny 始终优先。

### 24.2 CLI

目标 surface 可包括：

```text
rove --agent workspace:ops-diagnostic "task"
rove agents list
rove agents show <selector>
rove agents validate <selector-or-path>
rove procedures list
rove procedures validate [path]
rove procedures search <query> --agent <selector>
rove procedures index
```

命令名称不是本文的强制 API；需要保持的语义是可发现、可验证、可解释 selection，不要求用户靠试运行猜结果。

### 24.3 API

create-job request 可以显式提供：

- agent selector；
- execution strategy override；
- optional pinned procedure IDs；
- user constraints。

job/run state 应返回 resolved selector/profile hash、selected procedure metadata 和 degradation status。API 不默认返回完整 trusted policy/procedure body。

### 24.4 Config dump and diagnostics

redacted dump 应显示：

- resolved Agent source/ID/version/hash；
- instruction paths/hashes；
- procedure roots、trust policy、catalog generation；
- capability requirement resolution；
- fallback/degradation；
- 不显示 secret、完整 policy body 或 credential-bearing MCP config。

### 24.5 Web/terminal

UI 可以展示：

- active Agent name/version/source；
- selected procedures 与 why-selected；
- stale/degraded/missing capability；
- current procedure step reference；
- deviations；
- run 使用的 immutable hashes。

UI 不自行重新检索 procedure，也不根据标题猜 active Agent。

## 25. Testing Strategy

### 25.1 Schema and loader tests

- valid/invalid Agent manifest；
- stable canonical definition/profile hash；
- explicit source selector 与 no-shadowing；
- path traversal、symlink/junction escape；
- missing/oversized/non-UTF8 instruction；
- runtime compatibility；
- no-secret/no-executable-hook validation；
- legacy prompt mapping/fallback event；
- operator caps 与 Agent defaults 取交集。

### 25.2 Workspace instruction tests

- root `AGENTS.md` discovery；
- nested scope chain；
- target path 多规则；
- deeper-specific rule 与 root constraint；
- unreadable/conflicting instruction semantics；
- mid-run source change 不改变 pinned bundle；
- `AGENTS.md` 不进入 procedure catalog。

### 25.3 Procedure validation tests

- required frontmatter/body sections；
- trust derived, cannot self-claim；
- version/status/supersedes；
- platform/workspace/agent applicability；
- required capability resolution；
- risk/side-effect/rollback requirements；
- hard/soft expiry；
- declared parameters vs hardcoded environment values；
- broken links；
- dangerous snippet without risk metadata；
- prompt-injection and hidden-content fixtures。

### 25.4 Selection tests

- eligibility always precedes ranking；
- high semantic score cannot bypass platform/trust/capability/risk；
- exact intent/tag/path matching；
- no-match returns empty；
- same ID multi-version resolution；
- conflict/dedup/threshold；
- deterministic manifest fallback；
- score breakdown stability；
- query redaction；
- user pin still passes hard validation。

### 25.5 OnCall-inspired scenarios

使用重新编写的本地 fixtures，不直接假定 OnCall 示例环境：

1. “磁盘使用率过高”在 Linux + disk/log capabilities 下选择 diagnose procedure；
2. Windows workspace 排除 Linux-only procedure，即使 semantic score 最高；
3. 只有诊断目标时不 hydrate cleanup/delete sections；
4. remediation 请求仍触发 approval，不因 runbook 建议而自动执行；
5. hardcoded region 未声明为 parameter 导致 validation warning/error；
6. placeholder contact/internal link 不被报告为真实 escalation path；
7. retrieval backend error 与 empty result 被严格区分；
8. uploaded malicious Markdown 只能作为 untrusted reference，不能成为 selected procedure。

### 25.6 Lifecycle integration tests

- profile resolution -> strategy -> procedure selection -> plan event ordering；
- Planner plan refs pin procedure ID/version/hash；
- StepRunner hydration 只加载所需 sections；
- deviation 写入 StepRecord 并可触发 plan revision；
- Finalizer 不把 procedure adherence 当成功证据；
- selection revision 保留 parent 和已执行 facts；
- cancellation/timeout/profile error 只有一个 `run_completed`；
- resume 使用 exact profile/procedure snapshots；
- missing snapshot/latest source mismatch 不静默继续；
- self-authored candidate 在当前 run 不激活。

### 25.7 Evaluation metrics

procedure evaluation 至少包含：

- selection precision/recall；
- false-positive wrong-platform/wrong-capability rate；
- no-match correctness；
- stale procedure rate；
- procedure context token overhead；
- plan quality change；
- tool-call efficiency；
- adherence/deviation correlation with success；
- safety/approval bypass，目标为零；
- external-to-trusted privilege escalation，目标为零；
- deterministic vs vector/model selection cost and quality。

## 26. Risks and Trade-offs

### 26.1 Schema overhead

过重 frontmatter 会降低作者意愿。第一阶段应保留最小 required set，risk/freshness/capability 字段按场景要求；提供 validator 和模板，而不是靠文档记忆格式。

### 26.2 Instruction bloat

Agent policy、workspace rules 和 procedures 可能快速占满 context。通过 policy/default 分离、root instructions 简洁化、progressive hydration 和 section token accounting 控制。

### 26.3 False sense of trust

`workspace_trusted` 不代表 runbook 永远正确。需要 freshness、provenance、runtime evidence 和 tool safety共同约束。

### 26.4 Capability taxonomy drift

过细 capability IDs 会难维护，过粗又无法准确过滤。先覆盖稳定、高价值能力族，并允许 namespaced unstable binding；通过 MCP follow-up spec 收口。

### 26.5 Agent fragmentation

大量近似 Agent packages 会造成指令和 procedure 分叉。通过 explicit source selector、shared procedure roots、version lineage 和 evals 减少复制。

### 26.6 Procedure selection latency

pre-planning retrieval增加延迟。deterministic metadata/lexical baseline、manifest cache、top-k cap 和 `react` no-match fast path可以控制。

### 26.7 Snapshot storage

每个 run 保存 selected procedure snapshot 会增加磁盘占用，但换来可审计 resume。只快照选中内容、内容寻址去重和 TTL policy可后续优化。

### 26.8 Stale source vs reproducible resume

旧 snapshot 保证语义一致，却可能含已撤销建议。当前 runtime hard safety始终优先；严重撤销可以通过 revocation metadata 阻止 resume 并要求 restart。

### 26.9 Plain-text conflict detection is imperfect

自由文本 policy 冲突无法完全静态证明。高风险规则应尽量结构化，并由 runtime enforcement；诊断器只能辅助，不能被当作安全证明。

## 27. Acceptance Criteria

当且仅当以下条件都有实现、测试和 runtime 文档证据时，本文目标才可标记为 implemented：

1. 存在 schema-versioned、named、versioned AgentDefinition。
2. 每个 run 使用 immutable AgentRuntimeProfile，并记录 definition/profile hashes。
3. 当前 prompt 配置有明确 legacy profile compatibility path。
4. explicit invalid Agent selection 不被静默替换。
5. Agent policy 与 overridable defaults 被分开处理。
6. Agent package 不能携带 secret 或 executable auto-hook。
7. root `AGENTS.md` 至少可被发现、hash、注入和持久化；nested scope 有明确 contract/roadmap。
8. workspace instructions 不默认进入 procedure catalog。
9. procedure 使用 machine-readable metadata 和 human-readable body。
10. procedure trust 由 source/operator policy 派生，不能自报。
11. active mutating procedure 有 risk/effect、approval expectation、validation 和 rollback/non-reversible note。
12. diagnose/remediate/verify 能被区分，不因诊断请求自动选择 destructive section。
13. procedure 引用 stable capability IDs，runtime 生成具体 tool binding。
14. capability binding 不增加 operator permission，approval 仍生效。
15. procedure corpus 与 reference code/docs corpus 在 authority 和 selection 上隔离。
16. invalid procedure 不进入 active index，并有 validation report。
17. procedure index 有 content hash、version、provenance、freshness 和 generation identity。
18. eligibility filter 在 semantic/lexical ranking 之前。
19. wrong trust/platform/scope/capability/risk/freshness 候选不会因高相似度入选。
20. no-match、degraded 和 error 是不同 typed outcomes。
21. selection 有 safe score breakdown、catalog hash 和 selected content hashes。
22. PlannerContext 只接收 selected procedure summaries/outlines，不注入整个 corpus。
23. StepRunner 按 pinned hash hydrate 必要 sections，并记录 applied/deviation。
24. procedure、memory、reference 和 tool output 不被拼成 trusted policy text。
25. external uploaded content 不能自动成为 trusted procedure。
26. Agent 自行生成的 procedure 在当前 run 只能是 draft candidate。
27. profile、instructions、procedure selection/hydration/deviation 有 canonical events。
28. run artifacts/checkpoint/report 保存 exact Agent/procedure identity和必要 snapshots/pointers。
29. resume 不静默加载 latest definition/procedure 替代旧 hash。
30. 当前 runtime hard policy 在旧 snapshot resume 时仍优先。
31. config dump、API 和 UI 能展示 resolved identity/degradation且不泄漏全文/secret。
32. deterministic catalog/selection fallback 不依赖 remote model/vector backend。
33. procedure selection 和 lifecycle integration 有 deterministic fake-model tests。
34. [`docs/runtime/`](../runtime/README.md) 只在实现后更新，并继续作为当前事实来源。

## 28. Implementation Dependency Order

本文不是实现 checklist，但后续计划应按依赖顺序组织：

1. **Taxonomy and legacy profile**

   固定 authority classes、AgentDefinition/Profile types、legacy prompt mapping 和 content hashing。

2. **Package loader, validator, and runtime identity**

   先做到 definitions 可安全解析、诊断和固定，不接 procedure retrieval。

3. **Workspace instruction bundle**

   先实现 root `AGENTS.md`，再扩展 nested scope；同步 prompt metadata。

4. **Procedure schema, validator, source catalog, deterministic selection**

   先 metadata filter + lexical/tag/path，不依赖 vector/model。

5. **Capability IDs and binding**

   与 execution lifecycle capability snapshot 对齐，保持 approval/tool safety。

6. **Progressive hydration and lifecycle integration**

   接入 PlannerContext、StepRunner、deviation、PlanEvaluator 和 Finalizer。

7. **Artifacts, events, checkpoint, resume, report, repair**

   固化 exact profile/procedure identity 与稳定恢复语义。

8. **Interface and diagnostics**

   CLI/API/Web 暴露 selector、validate、search 和 resolved state。

9. **Vector/model ranking and evaluation**

   deterministic baseline 有评估后再启用增强路径。

每一阶段都必须同步 tests 和必要的未来/当前文档边界；不能在实现前把 `docs/runtime/implementation-status.md` 标成已完成。

## 29. Relationship to Existing and Future Documents

### 29.1 Existing docs

- [`2026-07-14-agent-execution-lifecycle-design.md`](2026-07-14-agent-execution-lifecycle-design.md) 消费本文定义的 profile、capability snapshot 和 procedural context。
- [`2026-05-24-rove-runtime-hardening-design.md`](../Archive/design/2026-05-24-rove-runtime-hardening-design.md) 是已归档的 local-first hardening 设计背景。
- [`2026-05-24-rag-pipeline-hardening-design.md`](../Archive/design/2026-05-24-rag-pipeline-hardening-design.md) 是已归档且其产品路径已被移除的 RAG 设计；只能参考工程原则，不能作为现有基础。
- [`docs/runtime/subsystems.md`](../runtime/subsystems.md) 与 [`docs/runtime/implementation-status.md`](../runtime/implementation-status.md) 继续描述真实的 prompt、memory、workspace retrieval 和 tool behavior。

### 29.2 Follow-up specs/documents

- [`2026-07-15-mcp-streamable-http-and-tool-artifacts-design.md`](2026-07-15-mcp-streamable-http-and-tool-artifacts-design.md)：能力、resource/artifact 和 transport metadata 的完整协议；
- [`2026-07-15-oncall-reference-agent-evaluation-plan.md`](2026-07-15-oncall-reference-agent-evaluation-plan.md)：对 baseline、procedure-aware planning 和成本/质量的评估；
- [根级 `AGENTS.md`](../../AGENTS.md)：已经作为维护者规则落地；runtime 的 scope discovery/typed instruction bundle 仍待实现；
- [维护者 onboarding](../ONBOARDING.md)：已经记录当前事实、入口、验证和文档地图；
- procedure authoring guide/template：在 schema 实现稳定后提供，不提前制造无法验证的格式承诺。

## 30. Design Decision

本设计的核心决定是：

> rove 的 Agent 不能只等于一段 system prompt，procedure 也不能只等于被向量检索命中的 Markdown。Agent 必须有可版本化、可校验、可固定的 runtime profile；procedure 必须有适用条件、trust、capability、risk、freshness、evidence、validation 和 provenance，并且始终低于 runtime policy、workspace policy 和当前用户约束。

最终边界是：

- AgentDefinition 定义稳定角色与受限 defaults；
- `AGENTS.md` 定义 workspace 规则；
- procedure 提供经过筛选的做事方法；
- reference RAG 提供事实资料；
- memory 提供历史偏好、反馈和决策；
- tool/StepRecord 提供本次运行证据；
- runtime 执行真正的 safety、approval、budget、event、artifact 和 resume contract。

借鉴 OnCall 的关键不是“把五份运维 Markdown放进向量库”，而是认识到 Planner 需要领域经验。rove 应把这一步做得更可验证、更可解释、更安全，也更适合长期演进。
