# 硬只读 Review 工作流设计

> Status: **Implemented in `feature/read-only-review`; pending merge and optional external gates**
>
> Date: 2026-08-16
>
> Base: `main` at `f6676d1`（worktree `feature/read-only-review`，基线 `5fe9d70`）
>
> 计划：[docs/plans/2026-08-16-read-only-review-workflow.md](../plans/2026-08-16-read-only-review-workflow.md)
>
> 本文是该计划第一阶段（"先写一份可评审设计"）的产出。实施以本文获确认为前提；
> 实施开始前在仓库根创建 `IMPLEMENTATION_LOG.md`。

> Current implementation note (2026-08-18): the Runtime target snapshot,
> Review execution profile, finding sanitization/finalization, API/ProductStore
> lifecycle, CLI entry, Web composer/Inspector, and persistence redaction are
> implemented in this worktree. The deterministic Rust and Web checks listed in
> `VERIFICATION.md` are the current evidence. Credentialed external Provider,
> third-party MCP, ConPTY, packaging/signing, and installed-Desktop gates remain
> optional and unrun; this record does not claim those integrations.

## 0. 已核对的现状（2026-08-16，基线 5fe9d70）

> **Implementation corrections (2026-08-16).** The first draft was useful for
> review but left safety details implicit. The implementation treats the
> following as normative wherever older prose differs: Git state is represented
> independently for HEAD/index/worktree; all Review reads use an immutable
> snapshot; raw model findings are never persisted as ordinary tool payloads;
> Review state lives outside the target workspace; unfinished runs are
> conservatively classified as `needs_attention` on ProductStore restart rather
> than replayed; dispatch authorization binds the pinned descriptor and effect
> class; Review read/search output is not retained as a durable Tool Artifact;
> and completeness is runtime-derived rather than model-claimed.

设计建立在对以下现有行为的核对之上（文件引用均为当前源码）：

| 领域 | 现状 | 关键引用 |
|---|---|---|
| Git diff | API 层有界 diff：git CLI、porcelain v1 `-z`、`GIT_OPTIONAL_LOCKS=0`、10s 超时、512 git 条目 / 4096 总条目 / 128KB 每条 / 4MB 总量、binary 检测、untracked 合成 patch；不区分 staged/unstaged、丢弃 rename 源路径、无 digest；runtime 层无 diff 模块 | `apps/api/src/product/diff.rs` |
| Artifact | `ToolArtifactStore`：run 目录持久化、put/get/metadata/ledger/expire、Sensitivity/Trust/Claim；API 暴露 list/content/download/preview；读类工具输出自动保留 | `runtime/src/state/tool_artifacts.rs`、`runtime/src/tools/executor.rs:102`、`apps/api/src/product/artifacts.rs` |
| Run Inspector | Web Inspector 面板（Artifact/Diff/Files/Export），dialog + focus 管理，数据来自 `WorkbenchState` 与 transcript projection | `apps/web/inspector/RunInspector.tsx` |
| ToolRegistry | `register/try_register/snapshot`（run 级 pin）、`descriptors()`；`ToolDescriptor { destructive, parallel_safe, capability_id, capability }`；mutation class 仅由 `destructive` 推导，无显式 read-only 属性 | `core/src/tools.rs`、`runtime/src/foundation/capability.rs:72` |
| Execution Environment | `ExecutionEnvironment` trait + `ExecutionCapabilities{filesystem_read, filesystem_write, process_run, process_stdio, observations, process_background, process_pty, workspace_checkpoints, artifact_projection}`；工具实现内部检查 capabilities（如 `fs.rs:103`、`search.rs:262`）；`LocalExecutionEnvironment::new` 全开，`InMemoryExecutionEnvironment::with_capabilities` 可受限；`EngineEnvironmentOptions` 按调用注入 env | `runtime/src/environment.rs`、`runtime/src/engine/facade.rs:125` |
| Project Trust | `ProjectTrustRepository` 持久授权、capability digest 校验、`ProjectActivationState`；受限时不读/不启 MCP（`registry.rs:107`）、workspace instructions 需独立 capability；revoke 触发 job 隔离 | `apps/bootstrap/src/project_trust.rs`、`apps/bootstrap/src/registry.rs`、`apps/api/src/product/trust.rs` |
| 审批 | Executor 管线：schema→校验→pre-hook→`check_tool_allowed(destructive×policy)`→执行→post-hook；交互审批在 `tool_turn`（Ask+destructive→provider），`approval_decision=Approve` 可放宽一次 Ask 策略 | `runtime/src/tools/executor.rs:57`、`runtime/src/workspace/boundary.rs:7`、`runtime/src/engine/tool_turn.rs:203` |
| Finalizer | 独立、有界、evidence-grounded：只读 facts、禁止工具调用、JSON `{"answer"}`、确定性 fallback、`FinalizationRecord` + `FinalOutcomeStatus` | `runtime/src/planning/finalizer.rs` |
| Engine/状态 | `run_with_cancel` 每 run snapshot registry + `CapabilitySnapshot`；`RuntimeIdentity` 持久记录 env capabilities 与 capability snapshot id（只读证明可追溯）；`StateStore` task_state/report/trace/resume；ProductStore schema v13；API job 监督（JobRecord/supervisor/panic 恢复/trust monitor） | `runtime/src/engine/facade.rs:481`、`runtime/src/foundation/runtime_identity.rs`、`runtime/src/state/store.rs`、`apps/api/src/product/store/schema.rs:9`、`apps/api/src/lib.rs:2136` |

结论：**两道边界（注册期目录 + 工具内 capability 检查）与 run 级 pin 的机制都已存在**，
缺的是（1）显式的 review 只读工具目录与环境变体，（2）dispatch 前的 review 模式再校验，
（3）runtime 层目标快照/digest，（4）版本化 finding 合同，（5）产品入口。

## 1. Review target 与 digest

### 1.1 目标种类（`ReviewTargetSpec`，用户显式指定）

| kind | 语义 | 解析 |
|---|---|---|
| `uncommitted`（默认） | HEAD vs 工作树（含 staged、unstaged、untracked） | `git status --porcelain=v1 -z --untracked-files=all` + 每文件 diff |
| `base <rev>` | 解析后的 base commit vs 工作树（覆盖 base 以来的已提交+未提交改动） | `git rev-parse <rev>^{commit}` 解析并记录 SHA |
| `commit <rev>` | 单个 commit 自身的变更（parent vs 该 commit） | 同上；diff 为 `<sha>^..<sha>` |

非 Repo 工作区请求任一目标 → typed failure `review_target_unavailable`（Folder 无 Git 事实）。
revision 无法解析、仓库缺失、git 不可用 → 同一 typed failure，带 reason，不落入通用 500。

### 1.2 目标快照（`ReviewTargetSnapshot`，启动时一次性采样，不可变）

对每个变更文件记录：

- `path`（仓库相对、UTF-8、保留 Unicode 原样）；
- `change_kind`：`staged | unstaged | untracked | deleted | renamed | binary | modified`
  （由 porcelain X/Y 码推导；X 和 Y 必须分别保留，不能压成一个状态）；
- `head_hash/index_hash/worktree_hash`：三个 Git 状态的完整内容 SHA-256（流式读取，
  不设置内容大小上限；读取/时间预算耗尽时显式记 `hash_truncated=true`）；
- `old_path`（rename/copy 时保留源路径）；
- `binary`：字节含 NUL 或 git binary patch 标记；
- `diff`：有界 unified diff 文本（每文件 ≤128KB，超长截断并记 `diff_truncated`）；
- untracked 文件内容按现有合成 patch 模式生成，同样有界。

快照捕获使用与 `diff.rs` 相同的防御：`git -C <root>`、`GIT_OPTIONAL_LOCKS=0`、
`--no-ext-diff`、10s 超时、总量上限（512 文件 / 8MB diff 总量，超限 → `entries_truncated`
计数并记入未检查范围）。`GIT_OPTIONAL_LOCKS=0` 保证捕获不写 `.git/index`。

### 1.3 Target digest

`target_digest = stable_hash(canonical_json)`，输入为：

```
{ workspace_digest, kind, resolved_base?, entries: [ {path, change_kind, old_path?,
  old_hash, new_hash, binary, truncated_flags} ... ] }   // 按 path 字节序排序
```

- `workspace_digest` 复用现有 `workspace_identity_digest`；
- digest 不含 diff 文本本身（diff 可截断），但含每侧内容 hash，因此**任何文件内容、
  路径集合或 base 变化都会改变 digest**；
- `resolved_base` 记录解析后的 commit SHA（不记录用户输入的原始 rev 字符串之外的解释）。

### 1.4 Stale 语义

- digest 在三个时点计算：**启动采样**、**finalize 前**、**结果读取时（有界惰性）**。
- finalize 前不一致 → 结论 `stale`，已提交 finding 保留但整份结果标记 stale，不静默归因；
- 读取时不一致（懒重算，缓存在 review 行，最小间隔避免每次 GET 全量扫描）→ 状态
  `needs_attention`，UI 显示"目标已变化，结果基于旧快照"并提供重新 Review 入口。

## 2. 只读执行合同

Review = 共享 Runtime 上的**受限运行 profile**。不新增第二套 Agent loop、Planner、事件
生命周期、ToolRegistry、token authority 或持久队列。

### 2.1 边界一：Review 专用工具目录（注册期）

`apps/bootstrap/src/registry.rs` 新增 `review_tool_registry(workspace)`，只注册：

| 工具 | capability_id | 说明 |
|---|---|---|
| `read_file` | `workspace.fs.read` | 从启动时物化的不可变快照有界读取 |
| `list_directory` | `workspace.fs.list` | |
| `glob_paths` | `workspace.search.glob` | |
| `search_code` | `workspace.search.text` | 截断结果仅进进程内 projection store |
| `repository_map` | `workspace.repository.map` | ignore-aware 确定性检索（workstream B） |
| `resolve_tool_artifact` | `runtime.artifact.read` | 读取本 run 已保留的 Artifact |
| `review_target_diff`（新） | `review.target.read` | 从**已采样的不可变快照**返回有界 per-file diff/status，不在 dispatch 时再跑 git |
| `review_submit_findings`（新） | `review.findings.submit` | 见 §3.3；唯一"写"，且只写受控 Rove state |

不注册：所有写文件/edit/delete/move、checkpoint 系列、全部 shell 系列、全部 memory 写、
`request_input`、`memory.topic.read`（首版最小集）。**MCP 一律不注册**（含 trusted 工作区）。
hooks 使用空 `HookRegistry`（不加载 workspace hooks）。

### 2.2 边界二：只读 Execution Environment（能力期）

`LocalExecutionEnvironment::read_only(workspace)`：

```
filesystem_read = true      filesystem_write = false
process_run = false         process_stdio = false
process_background = false  process_pty = false
observations = false        workspace_checkpoints = false
artifact_projection = true  // 仅进程内 TransientArtifactStore（内存），供 search 截断投影
```

现有工具实现内部已检查这些 capabilities（`fs.rs:103`、`search.rs:262`、`coding.rs:1490+`），
因此即使某写工具意外进入目录，环境层也会以 `CapabilityUnavailable` typed failure 拒绝。

### 2.3 边界三：dispatch 前 review 模式再校验（每次调用）

`Executor::run_with_input_events` 的权限边界处（现有 Step 4 附近）增加：当
`RuntimeToolServices.run_mode == Review` 时，`descriptor.capability_id` 必须属于
`runtime::review::REVIEW_ALLOWED_CAPABILITIES`（§2.1 的 8 个常量集合），否则返回
`ToolError::PermissionDenied { reason: "review mode forbids non-read-only tool" }`，
并记录 security event（trace 中可见）。该校验在 schema 校验之后、执行之前，与
approval 检查同级。

三道边界相互独立：提示词、模型输出、仓库 `AGENTS.md`、MCP annotation、URL、文件名、
approval 都无法在任何一层把只读集合扩大——因为 allowlist 是编译期常量集合，
且 `RuntimeToolServices.run_mode` 由 Engine 构造时固定，非工具可触达的数据。

### 2.4 审批交互（明确不可授权）

- Review run 的 `approval_policy` 固定投影为 `Never`（对 destructive）且 destructive
  工具本就不在目录中 → 不会产生审批请求事件；
- 不设置 `approval_provider`；即使调用方强行设置，`check_tool_allowed` 与 §2.3 的
  allowlist 都不读 approval 结果，**用户点击批准也无法获得写权限**；
- 用户全局/会话的 approval 偏好被记录但被 review 合同覆盖（响应中说明覆盖原因）。

### 2.5 Shell

首版 **shell 零可用**（不注册、环境 process_run=false、allowlist 不含）。计划中
"参数化只读命令 allowlist" 不在本版实现；如后续需要，必须作为独立合同变更并带证明。

### 2.6 允许的自身写入（受控 Rove state）

Review run can write its own external run directory (`trace.jsonl`,
`task_state.json`, `report.json`, and the sanitized `review.json`) and the
ProductStore Review projection through the API supervisor. The target
snapshot, including bounded captured source bytes, is stored separately under
the external Review state root. Read/search output is deliberately not copied
into the durable `ToolArtifactStore`; all of these writes remain outside the
target workspace and cannot change the target snapshot.

## 3. Finding 与结果合同

### 3.1 版本化 Result schema（`review_result_schema_version = 1`）

```jsonc
{
  "schema_version": 1,
  "review_id": "rev_...", "run_id": "...", "session_id": "...",
  "target": { "kind": "...", "resolved_base": "...", "digest": "...", "entries": <bounded>,
              "workspace_digest": "...", "captured_at": "..." },
  "conclusion": "pass|findings|partial|stale|unavailable|cancelled|error",
  "findings": [ <finding> ... ],                 // ≤64 条，超出记 truncated_findings
  "stats": { "files_scanned": 0, "bytes_scanned": 0, "duration_ms": 0,
             "concurrency_limit": 0, "findings_total": 0, "truncated_findings": 0 },
  "unchecked": [ { "reason": "diff_truncated|binary|hash_truncated|entries_truncated|...",
                   "paths": [ ... ] } ],
  "model_snapshot": { /* 现有 RunModelSnapshot 的 secret-free 投影 */ },
  "capability_snapshot_id": "...",               // 现有 CapabilitySnapshot id
  "execution_environment": { /* 现有 ExecutionEnvironmentIdentity+capabilities 投影 */ },
  "warnings": [ "..." ]
}
```

`conclusion` 与 run 状态分离：`pass` 仅表示"完成且无 finding"，绝不等于"所有文件已
检查"——`unchecked` 与 `stats` 独立呈现。

### 3.2 Finding schema

```jsonc
{
  "finding_id": "rfd_<stable_hash>",             // = hash(review_run_id, dedup_key, index)
  "severity": "critical|high|medium|low|info",
  "confidence": "high|medium|low",               // 桶枚举，不接受伪精度数字
  "category": "<≤64 chars>",                     // 建议词表写在 prompt，不强制封闭集
  "path": "<workspace 相对路径>",
  "location": { "start_line": 0, "start_col": 0, "end_line": 0, "end_col": 0 },
  "location_status": "validated|unvalidated|invalid",  // 见下
  "title": "<≤200 chars>",
  "explanation": "<≤4KB>",
  "evidence": [ { "snippet": "<≤2KB>", "source": "diff|file|artifact", "ref": "..." } ], // ≤3
  "rule": "<触发规则或来源，≤200 chars>",
  "suggestion": "<≤2KB>",                        // 纯文本建议，无任何自动应用
  "status": "open"
}
```

### 3.3 Finding 的产生与校验管线

1. **分析阶段**：模型用 §2.1 读工具 + 快照 diff 分析，以
   `review_submit_findings`（terminal 工具，schema 在 dispatch 期强校验）一次性提交
   finding 列表。首版字段上限在工具 schema 内声明（maxLength/maxItems），复用现有
   `validate_args` 硬校验。
2. **提交语义**：每次 run 至多一次成功提交（重复调用 → typed failure
   `review_findings_already_submitted`）。原始参数只在进程内 bounded staging buffer 中存在；
   canonical events、trace、普通 Tool Artifact 和 report 只写经过 schema 限制、路径校验和
   脱敏后的 finding/reference，不写 raw prompt、隐藏推理或完整 payload。
3. **Review Finalizer**（复用现有 Finalizer 模式的独立阶段，无工具、有界、可确定性
   fallback）：输入 = 提交的 raw findings + run 的有界执行事实（StepRecords/预算）+
   目标快照；执行——
   - schema/字节上限校验（超限字段截断并计入 warnings，不静默丢弃整条）；
   - **路径校验**：path 必须规范化为 workspace 相对路径且解析后位于 root 内（复用
     `boundary.rs` 归一化）；不在目标变更集合内 → 保留但加 warning
     `finding_outside_target`；
   - **行号校验**：对快照中已捕获内容的文件，行/列必须在真实范围内，否则
     `location_status=invalid`（finding 保留、如实标记，不猜测修正）；
     二进制/截断/未捕获文件 → `unvalidated`；
   - **去重**：dedup key = (path, category, start_line, lowercase(title) hash)；
   - **脱敏**：snippet/explanation 过敏感路径与 secret 赋值模式（对齐
     `local_tool_output_sensitivity` 与 CDH export 的 redaction 词汇，提取为 runtime
     共享 helper），命中即整段替换为 `[redacted]` 并计数；
   - **结论合成**：规则优先（无提交→partial；有未检查范围→独立呈现；digest 变化→
     stale；取消→cancelled），不依赖模型自述。
   模型不可用/校验全败 → 确定性 fallback = `partial` + warnings，**绝不虚构 finding**。
4. finding 是不可信模型输出：raw 提交（含原始 system prompt、隐藏推理、完整 tool
   payload）只在进程内的有界提交阶段存在；canonical events、trace、report、导出和
   浏览器状态只接收经过脱敏的投影，不进入 result 或普通 Artifact。

### 3.4 Authority 与派生投影

- **权威**：run 的 canonical events + task_state.json + report、外部
  `ReviewTargetSnapshot`/sanitized `review.json` 和 ProductStore 的 Review/finding
  rows；Review 禁用 read/search Tool Artifact retention；
- ProductStore 行（§5.1）是同一结构化 result 的受控产品投影；
- API/Web/CLI 的所有严重程度、统计数字均来自同一结构化 result，不做本地再猜测。

## 4. 运行、取消与恢复

### 4.1 运行载体

- Review run = 普通 Engine run（unplanned 或 planned 均可，首版 unplanned +
  Finalizer 阶段），复用 `RunId/JobId/SessionId`、canonical events、StateStore、
  trace/report、budget（`ExecutionPolicy` 维度照常生效并记录在 result.stats）。
- API 侧复用现有 job 监督（JobRecord/supervisor/panic 恢复/trust monitor），但
  **不占用 product-turn claim**、不进入聊天消息 transcript；review 有自己的
  单飞行状态（每 session 同一 target digest 只允许一个 active review，重复请求
  幂等返回现有 review_id + 200）。

### 4.2 取消

- API `POST /product/reviews/{review_id}/cancel` 与 CLI Ctrl-C：传播现有
  `CancellationToken`；无成功提交 → 结论 `cancelled`；已有提交 → 结论 `partial`
  （已提交部分如实呈现）+ cancelled 标记。取消后不发布新 finding。

### 4.3 断线重连与恢复

- Web 刷新/断线后由 ProductStore 行恢复 review 状态（GET 幂等）；v1 用有界轮询
  （对齐现有 background status polling 模式），SSE reattach 为后续项；
- 进程重启：Review 行、外部 state root 下的 `review.json`、target snapshot、sanitized
  finding projection 和 canonical lifecycle facts 一起保留。当前实现把仍处于
  `queued`/`running` 的行统一标记为 `needs_attention`，不自动重新启动模型或重放
  工具；已完成结果通过 CAS 幂等读取，绝不重复发布 finding。无法证明执行完成时采用
  保守的 attention 状态，而不是猜测成功。
- **不重复发布**：finding 发布以 ProductStore Review 行的 result CAS 为闸，finding
  以稳定 `review_id`/`finding_id` 主键投影；已完成结果不会重新发布。

### 4.4 并发与预算

- 多 review 并发：各自独立 run + 独立 run 目录；registry/env 按 run 实例化，无共享
  可变状态；`ExecutionPolicy` 预算与并发上限照常生效并记录；
- 同 session 不同 target 可并行；同 target 并行被 §4.1 幂等规则折叠。

## 5. 产品入口

### 5.1 API（版本化、OpenAPI 注册于现有 `OpenApiRouter`）

| Method | Path | 说明 |
|---|---|---|
| POST | `/product/sessions/{session_id}/reviews` | 启动；body `{target: {kind, base?, commit?}}`；幂等（同 active target 返回现有） |
| GET | `/product/sessions/{session_id}/reviews` | 列表（含状态、digest、结论摘要） |
| GET | `/product/reviews/{review_id}` | 状态 + result 摘要（含 unchecked/warnings/stale） |
| GET | `/product/reviews/{review_id}/findings?limit=&cursor=` | 分页 finding（顺序稳定：severity→path→line→finding_id） |
| POST | `/product/reviews/{review_id}/cancel` | 取消（幂等：terminal 后 200 + 当前结论） |

错误码走现有 `ProductErrorCode` 模式新增：`review_target_unavailable`、
`review_conflict`、`review_unavailable`。Review 不修改任何既有会话/消息路由。

### 5.2 CLI

`rove review [--base <rev> | --commit <rev>] [--format text|json|jsonl] [--max-steps N]`

- 默认 text 人类输出（结论、finding 表、未检查范围、digest）；`--format json` 单文档
  stdout；`--format jsonl` 每 finding 一行（首行 result 头）；
- 与现有全局选项（`--model`、`-C`、`--trust-project` 无效并提示——review 不需要
  project trust 授权，因为不加载 project config/MCP）一致；
- 退出码：0 = pass/findings 完成；2 = partial/stale/unavailable；3 = error；130 = 取消。

### 5.3 Web

- 入口：session 页 composer 区 "Review" 动作（弹出 target 选择：Uncommitted / Base… /
  Commit…），确认后创建 review；
- 展示：Inspector 新增 `ReviewPanel`（与 Files/Artifact/Diff/Export 并列 tab）——目标
  摘要（kind、resolved base、digest、captured_at、entries 计数）、状态/结论、finding
  列表（severity 图标、标题、路径:行 精确跳转至现有文件预览、展开 evidence/suggestion）、
  unchecked/warnings 区块、取消/重跑按钮；主聊天 transcript 不插入 review 消息（v1
  刻意边界，面板内呈现全部状态）；
- 状态覆盖：loading、empty（无 review）、running（含预算/进度事实）、pass（no
  findings）、findings、partial、stale/needs_attention（含"目标已变化"提示 + 重跑）、
  restricted（workspace 非 Repo 等）、unavailable、cancelled、error、retry；
- 长路径截断省略、窄窗口不重叠、键盘 focus 圈闭（复用 Inspector dialog 模式）、
  aria 语义、深浅主题——沿用现有面板的样式与测试基线。

### 5.4 TUI

首版不新增 TUI Review 界面；TUI 可后续通过同一 API/命令接入（不做私有 backend）。

## 6. 持久化与迁移

- ProductStore **schema v14**（additive）：`product_reviews`（id、session_id、workspace
  绑定、target spec+digest、status、conclusion、run_id、captured_at、finalized_at、
  idempotency key、unchecked/warnings 计数）与 `product_review_findings`（投影行：
  finding_id、review_id、排序键、finding JSON、location_status）+ 索引；严格/幂等迁移
  沿用 v13 模式；旧行为不变；
- Runtime state：不新增 schema 版本；review 事实分属现有 task_state（run 身份/身份里的
  env capabilities 与 capability snapshot id 即**只读配置的持久证明**）、trace、report
  （tool_mutations 为空即无 workspace 变更的又一证据）与 artifact store。

## 7. 验证计划（真实命令记入 IMPLEMENTATION_LOG.md）

定向 → 扩大：

1. `cargo test -p rove-runtime`（新 review 模块单测：target 捕获/分类/digest/stale、
   只读 env、executor 门禁、finding 校验/去重/脱敏/行号、finalizer fallback）；
2. `cargo test -p rove-integration-tests --test tool_safety` + 新 `--test review`
   （模型尝试写/删/`git reset`/任意 shell/MCP 写/越界路径/提示词授权 → 全部 typed
   拒绝且留痕；Review 前后目标树/index/配置/memory/Provider 的 digest 对比不变）；
3. `cargo test -p rove-integration-tests --test api`（review 路由合同、幂等、分页、
   OpenAPI 字段、v14 迁移）；
4. `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
   && cargo test --workspace`；
5. `apps/web/`：`pnpm test && pnpm typecheck && pnpm build`；受影响浏览器流程跑
   mocked spec；`local-full` fake-provider 场景按 integration-testing 门执行；
6. Fake Provider 用 `with_turns` 脚本化：确定 finding、无 finding、格式错误、超长输出、
   恶意工具调用五类；外部 Provider / 真实第三方 MCP gate 不运行并如实声明。

## 8. 本设计不做 / 边界

- 不做自动修复、批量 patch、auto-commit/checkout/回滚、"发现后自动写"；
- 不做 shell allowlist（§2.5）、不做 review SSE reattach（§4.3）、不做 TUI 界面（§5.4）；
- 不读取/修改/合并 `.worktrees/user-state-migration` 的任何未提交内容；所需 state/path
  能力一律走 `StateStore`/`ProductStore`/`Workspace` 公开接口；若其迁移先合入 main，
  本任务 rebase 后再合并，共享文件（`apps/api/src/lib.rs` 路由注册、bootstrap
  assembly）的改动保持最小并记录依赖；
- 文档交付：`REVIEW_WORKFLOW.md`、`SUMMARY.md`、`IMPLEMENTATION_LOG.md`、
  `VERIFICATION.md`、`DIFF_SUMMARY.md` + `docs/runtime/` 同步更新。

## 9. 计划落地的文件触点（预估，实施日志中按实际修正）

| 区域 | 文件 |
|---|---|
| runtime 新模块 | `runtime/src/review/{mod,contract,target,digest,redaction,finalizer}.rs`、`runtime/src/tools/review_{target_diff,submit_findings}.rs` |
| runtime 修改 | `environment.rs`（read_only 变体）、`tools/executor.rs` + `tools/runtime_context.rs`（run_mode 门禁）、`lib.rs` 导出 |
| bootstrap | `registry.rs`（review_tool_registry）、`assembly.rs`（review Engine 装配入口） |
| API | `apps/api/src/product/review.rs`（新）、`store/schema.rs` v14、`store/repository.rs`、`lib.rs` 路由注册 |
| CLI | `apps/cli/src/cli/{args,mod}.rs`、`apps/cli/src/cli/review.rs` |
| Web | `lib/rove-{types,client,state}.ts`、`inspector/ReviewPanel.tsx`、session 页入口、样式与测试 |
| 测试 | runtime 单测、`tests/review.rs`、`tests/tool_safety.rs` 增补、`tests/api.rs` 增补、web specs |
