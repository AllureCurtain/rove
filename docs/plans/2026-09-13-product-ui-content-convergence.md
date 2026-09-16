# 产品界面内容收敛与双语化计划

> Status: **Proposed / Not Implemented**
>
> Date: 2026-09-13（初稿）· 补充决策见 §0.2
>
> Base: `main` at `b652ec7`（docs: add architecture-walkthrough series）
>
> 关联文档：
>
> - [HTML 界面原型](../design/mockups/2026-09-13-product-ui-preview.html)（信息架构 + 暖米色/冷色精修双方案可切换，评审用）
> - [UI V3 重塑实现文档](../design/2026-08-24-frontend-ui-v3-restyle-implementation.md)（视觉候选之一：暖米色，待 Phase 2）
> - [产品 UI V2 精修设计](../design/2026-07-27-product-ui-v2-finesse-design.md)（视觉候选之二：冷色精修，历史参照）

## 0. 已确认的产品决策

### 0.1 2026-09-13 讨论记录

1. **视觉方向**：先各出一版对比再定——将同时产出「V3 暖米色（cc-partner/Anthropic 风）」与「现有冷色调精修」两版对比，对比后再定稿。本文档不预设视觉结论。
2. **内部信息处理**：**彻底移除出界面**，不设开发者模式开关。原始 ID、canonical 事件、诊断细节一律不在产品 UI 渲染；诊断事实继续由 `trace.jsonl`、`task_state.json`、`report.json` 与 API 承载，界面移除不损失任何数据（与架构不变量一致：trace 记录事件事实，report 是派生摘要）。
3. **推进顺序**：先完成内容收敛（Phase 1），再做视觉两版对比（Phase 2）。先定「给用户看什么」，再定「长什么样」，避免给即将移除的信息做皮肤。

### 0.2 补充决策（i18n 与开放问题拍板）

1. **双语与多语言**：产品 UI 需支持**中文与英文**，之后还要能扩展更多语言。实现方式为**轻量字典层**（集中式 locale 字典 + 简单 `t()` 消费，不引入重量级 i18n 框架、不新增 npm 运行时依赖）。
2. **i18n 时机**：与内容收敛**同一阶段落地**。理由：本轮本就要重写几乎全部界面文案，若先写死英文再抽字典，文案等于过两遍。Phase 1 交付时：产品路径文案全部走字典，默认语言 **中文**（软著演示优先），英文词条同步齐全；语言切换入口放在 Settings（最小实现：两项选择 + 本地持久化）。
3. **Fake provider**：改名**「本地演示 / Local demo」**并保留在产品表单（无密钥离线演示/验收有价值），附说明「内置模型，无需联网与密钥」。
4. **错误码**：**保留**短错误码（如 `boot-failed`），放次要位置（小字/次要行）；主体是人话标题 + 一句原因 + Retry。

## 1. 背景与目标

产品 UI（`apps/web`，Desktop 经 `desktop-dist` 静态导出复用同一实现）目前把大量面向开发者的诊断信息直接呈现给最终用户：原始持久化标识符、canonical 事件流水、prompt hash/cache key、工具风险等级、内部架构术语（ProductStore、Execution Environment、continuity 等）。文案全部硬编码英文，仓库无 i18n 层。这在开发者工作台语境下合理，但对最终用户是噪音，也削弱软著验收与演示时的产品完成度。

目标：

1. **产品界面只讲用户语言**——用户在默认路径上看到的每个词，都应该是「我正在做什么、进行到哪、结果如何」。
2. **文案一次改到位并可双语**——收敛后的文案落入中英字典，默认中文，为后续多语言扩展留好结构。

## 2. 范围

### 范围内

- `apps/web` 产品路径的呈现层文案与信息架构（TSX 内硬编码文案 → 字典键）。
- 轻量 i18n 字典层：`zh-CN` / `en-US` 两套词条、默认语言、Settings 语言切换与本地持久化。
- `/dev/*` 路由的生产构建门禁。
- 受影响的 Playwright e2e、Vitest 单测与 `docs/runtime/` 同步更新。

### 范围外

- 视觉样式（Phase 2 另行处理，本文档不做任何 token/样式改动）。
- API/OpenAPI 契约、Rust runtime、ProductStore——API 继续返回现有字段，前端只是不再渲染其中开发者字段。
- Tauri 原生壳、CLI/TUI 界面。
- 业务逻辑与状态流（`state/`、`product/`、`lib/` 的行为不变，仅消费方式变化）。
- 服务端按语言协商、URL locale 前缀、翻译平台——多语言扩展时再评估。

## 3. 改动总原则

- **移除优于折叠**：对最终用户无意义的信息直接不渲染，而不是藏进折叠区。
- **人话优先，保留可诊断性底线**：错误态给人话描述 + 次要位置短错误码，完整细节仍在 trace 文件与 API 响应中。
- **文案改动必须与 e2e 断言同 PR 更新**：现有多套 e2e 直接断言 UI 文案（见 §6），不允许先改文案后补测试。
- **产品路径禁止硬编码用户可见文案**：新增/改写的界面字符串一律进字典；`/dev/*` 开发页不在约束内。
- **不动 DOM 结构与类名**（除非条目明确要求），为 Phase 2 换肤保持稳定目标。

## 4. 逐项改动清单

### P0 轻量 i18n 字典层（新增，与 P1–P3 同 PR 或前置小 PR）

目标：给产品路径一个可扩展的双语消费方式，不引入新依赖。

| 项 | 规格 |
|---|---|
| 目录 | `apps/web/copy/`：`zh-CN.ts`、`en-US.ts`、`index.ts`（locale 类型、字典注册、默认值）、`use-copy.ts`（React hook / context） |
| 键结构 | 按域分组的扁平键，如 `inspector.run.summary`、`settings.provider.localDemo`、`chat.responding`；**键名稳定后不轻易改**（e2e 若需断言可断言人话，不断言键名） |
| 词条内容 | 人话文案；带插值的用简单函数或 `{{name}}` 占位（自研极简插值，不引 i18next） |
| 默认 locale | `zh-CN`；可用 `ROVE_DEFAULT_LOCALE=en-US` 覆盖（构建期 env，不新增 `NEXT_PUBLIC_*` 亦可：放 server 注入或直接读 `process.env` 的 client-safe 常量，实施时选最简单可测路径） |
| 持久化 | `localStorage` key（如 `rove.locale`），Settings 两项选择「中文 / English」 |
| SSR/hydration | 优先 `zh-CN` 默认字典渲染，避免 locale 未决时闪英文；切换语言整页文案即时替换（context 更新即可，无需路由） |
| 验收 | 产品路径无用户可见硬编码英文残留（允许品牌名 `rove`、技术标识符如文件名/模型 ID）；切换语言后 Settings/Chat/Inspector 主要文案均切换 |

说明：本层只服务**界面文案**。模型返回内容、工具输出、用户输入不翻译。

### P1 RunInspector 内容重做（`apps/web/inspector/RunInspector.tsx`）

「Run」标签页（入口按钮现名 **Evidence**）是重灾区，按下表处理：

| 现状内容 | 处理 |
|---|---|
| product session id / run id / active job id / resumed-from run id / turn ordinal | **移除**。会话身份由侧栏与 URL 承载，用户无需看到内部标识符 |
| event count、last signal、restore status 明细 | **移除**；恢复异常改为一条人话警告（见下） |
| Prompt hash / Cache key（`<code>` 短 id） | **移除** |
| prompt compaction 明细（degraded fallback、compacted message counts、partial_reasons） | **移除**；仅在发生降级时显示一行「为腾出空间，较早的对话已自动精简」 |
| Canonical events 原始事件列表 | **移除**。运行过程以「活动时间线」呈现（见下） |
| 工具列表附 `read only / mutation capable; <risk_level> risk` | **改写**：仅列本次运行用到的工具名，不显示能力/风险标注 |
| Evidence references（artifact/evidence 引用串） | **移除**；产物走 Artifacts 标签页 |
| "Durable identity" / "Observed live" continuity 标签、pricing snapshot 脚注 | **移除** |
| Workspace changes 变更清单 | **保留**（这是用户关心的「这次改了哪些文件」），文案改为「本次改动的文件」 |

替代内容——「Run」页重做为**运行摘要**（可与 HTML 原型右侧 Inspector 对照）：

- 状态行：排队 / 运行中 / 等待批准 / 已完成 / 失败（复用现有 run state，不新增后端字段）；
- 模型名、开始时间与耗时、token 用量与费用（均为 CDH 已有 usage/cost 数据的现有来源）；
- 活动时间线：由现有 events 派生的少量人类可读节点（开始、工具调用摘要、等待批准、完成/失败），**不渲染**原始事件 payload；
- 异常态人话文案：恢复不完整时「部分较早的消息未能恢复，完整记录已保存在本地」。

入口按钮 "Evidence" 改名 **Activity**（中文「活动」）。Files / Diff / Artifacts / Review / Export 面板为用户可理解功能，本项不动其逻辑，仅参与 P2 的文案通查与入典。

### P2 对话流与外壳文案（`apps/web/chat/`、`apps/web/shell/`）

| 位置 | 现状 | 改为 |
|---|---|---|
| `chat/Composer.tsx` + `shell/ProductApp.tsx` 的 resumeLabel 行 | "continuity: exact product session" / "first turn: server-bound session" | **整行移除** |
| `chat/Transcript.tsx` 消息署名 | "canonical message" / "responding" | 角色署名 + 时间（如「助手 · 14:32」/ "Assistant · 14:32"）；流式中显示「正在回复…」/ "Responding…" |
| Transcript 恢复文案 | "Reading canonical run events for this session." / "Available canonical events are shown. Some durable history could not be rebuilt." | 「正在恢复此会话…」/「部分较早的消息未能恢复，新消息不受影响。」 |
| Transcript 工具结果备注 | "No inline Diff was included in this canonical result." | 移除该行（Diff 面板的空态已自说明） |
| ProductApp 发送禁用原因 ×4 | "Restoring canonical history before a new turn." 等机器话 | 「正在完成会话恢复…」「会话设置加载中…」等短语，均 ≤6 词（中文对应更短） |
| ProductApp boot / route 错误视图 | 直接 dump 原始 `state.error` | 人话标题 + 一句原因 + Retry 按钮；**短错误码以次要样式展示**（如小字 `boot-failed`）；原始错误串不渲染 |

### P3 Settings 收敛（`apps/web/settings/`）

| 位置 | 现状 | 改为 |
|---|---|---|
| Provider 表单 | "API key env name" 顶层字段 | 移入表单内折叠的 Advanced 披露区，标签改「密钥环境变量名」；功能不变 |
| Provider 类型下拉 | 含 "Fake" | 改名**「本地演示」/ Local demo**，附说明「内置模型，无需联网与密钥」 |
| Provider 连接测试结果 | `Test: {status} · key_present=… · models=… · wire {wire_protocol}` | 「已连接 — N 个模型可用」/「密钥缺失」/「无法连接该服务」；不显示 wire 协议与布尔诊断 |
| Provider profile 摘要行 | `label · apiBase · env KEY_NAME` | `label · apiBase` |
| `RuntimeSettings.tsx` 章节标题 | "Connection" / "ProductStore" / "Execution environment" / "Agent runtime" / "Resume health" | 「连接」/「数据存储」/「运行时」/「会话恢复」（四个，合并重复语义） |
| `sections.ts` "Advanced / Developer" | 宿主 Benchmark runner | **整节移除**（benchmark 经 `rove-bench` CLI/API 使用；见 P4） |
| Export 面板入口文案 | "Evidence export" | 「导出对话（脱敏）」/ Export conversation (redacted)；JSON/HTML/MD 能力保留 |
| 新增「语言 / Language」 | （无） | 两项：中文 / English；写入 `rove.locale`，与 P0 一致 |

### P4 dev 路由门禁与 benchmark 迁出

- 新增构建态门禁：`app/dev/` 下 layout/page 读取 `process.env.ROVE_ENABLE_DEV_ROUTES === "1" || process.env.NODE_ENV === "development"`；不满足时渲染 `notFound()`。效果：`next dev` 与 e2e 环境照常可用，`next build`（含 `build:desktop` 产物 `desktop-dist`）不可达。仓库当前无任何 `NEXT_PUBLIC_*` 用法，此门禁为首个构建态开关，实现走 server component 读取即可，无需新增依赖。
- `tests/e2e/playwright.config.ts` 的 `webServer` 命令保持 `next dev`（开发态默认启用，无需改 env）；`workbench.spec.ts` 与 `real-api.spec.ts` 的 advanced smoke 不受影响。
- Benchmark runner 从 Settings 移除后，`components/benchmark-panel.tsx` 及其引用一并删除；CLI（`apps/bench`）与 API 入口不变，`docs/runtime/benchmark-evidence.md` 补一句「Web Settings 内的 runner 已于本轮移除」。
- 已知限制：`/dev` 路由的 JS chunk 仍会进入生产包（静态导出无法按路由条件剔除），但无用户可达入口；构建期彻底剔除列为可选后续项，不阻塞本轮。

### P5 防回归断言

- **收窄范围**（避免整页 regex 被用户输入/模型输出误伤）：在**产品 chrome** 上断言——对 `.inspector`、`.settings`、`.chat-composer`、`.product-topbar`、`.chat-transcript-frame`（或实施后等价容器）的 `textContent` 不匹配 `/product_session|run_id|job_id|prompt hash|cache key|canonical |continuity:|key_present|wire /i`。**不对整页/模型消息体**做该断言。
- 默认 `zh-CN` 下，同一断言另跑英文 locale（切 `rove.locale` 后）或在字典单测里断言 `en-US` 关键键无内部术语。
- `shell.spec.ts` 的 benchmark 断言（84-101 行区域）反转为「Settings 中不存在 Benchmark runner」。
- P0 增加 Vitest：字典键在 `zh-CN`/`en-US` 对齐、无空串；常用组件文案来自字典。

## 5. 明确不改动（边界记录）

- API 响应字段与 OpenAPI schema：`product_session_id`、`job_id`、`prompt_cache_key` 等继续存在，仅 UI 不渲染。
- `trace.jsonl` / `task_state.json` / `report.json` 的内容与格式。
- Inspector 的 Files / Diff / Artifacts / Review / Export 面板**行为**。
- `product/`、`state/`、`lib/` 模块的业务逻辑。
- 样式文件（`tokens.css`、`product.css`、`product-v2.css`）——Phase 2 的事。

## 6. 测试影响清单

| 测试 | 影响 |
|---|---|
| `tests/e2e/shell.spec.ts` | benchmark 断言反转；inspector 相关断言随 P1 改写 |
| `tests/e2e/settings.spec.ts` | 环境变量 label、本地演示类型、连接测试文案、RuntimeSettings 标题、语言切换断言更新 |
| `tests/e2e/continuity.spec.ts` | 恢复/部分恢复文案断言更新（默认中文文案；job_id 仅出现在 API mock 数据中，不属 UI 文案） |
| `tests/e2e/workbench.spec.ts` | 无预期变化（dev 态门禁默认放行） |
| `tests/e2e/polish.spec.ts` / `migration.spec.ts` / `product-ui-v2.spec.ts` | 预期无变化，回归确认；若存在英文文案硬断言则随默认语言改为中文断言 |
| Vitest 单测（`lib/*.test.ts`、`state/*`、`copy/*`） | 新增字典键对齐与插值测试；组件测试若断言英文文案则改为默认中文或显式注入 locale |
| 新增 P5 chrome 防回归 e2e | 产品容器无内部术语 |

## 7. 文档同步（同一 PR 内完成）

- `docs/runtime/` 中描述 Web 产品界面/Inspector/Settings 的 current-state 段落按新行为更新（含 implementation-status.md 中 Web 相关条目；注明默认界面语言为中文、支持 en-US）。
- UI V3 文档（Phase 2 输入）追加一行基线注记：其 §2 基线盘点写于本轮之前，换肤对象以本轮落地后的 DOM 为准。
- 本文档状态在实施完成后改为 Implemented，并附验证命令记录。

## 8. 执行顺序、工作量与工作流

### Phase 1 — 内容收敛 + 双语字典（本计划主体）

| 阶段 | 内容 | 估时 |
|---|---|---|
| 1 | P0 i18n 字典层 + Settings 语言切换 | 0.5–1 天 |
| 2 | P1 RunInspector 重做 + 对应 e2e | 1–1.5 天 |
| 3 | P2 对话流/外壳文案入典 + 对应 e2e | 0.5–1 天 |
| 4 | P3 Settings 收敛入典 + 对应 e2e | 0.5–1 天 |
| 5 | P4 门禁与 benchmark 迁出 + P5 断言 | 0.5 天 |
| 6 | 文档同步、全量回归（`pnpm test` / `typecheck` / `build` + e2e）、desktop-dist 重建 | 0.5–1 天 |

合计约 **4–6 人日**（含 i18n）。

工作流：在独立 worktree 进行；**改动先留工作区，用户本地确认效果前不 commit**；启动 dev server / 运行测试前征得同意；每阶段完成汇报一次。

### Phase 2 — 视觉两版对比（本文档之后、独立计划）

- 输入：Phase 1 定稿后的 DOM/文案 + HTML 原型中的暖米色/冷色精修两方案 + [UI V3 重塑文档](../design/2026-08-24-frontend-ui-v3-restyle-implementation.md) 与 tokens 附录。
- 产出：两版可运行对比（或截图），定稿后再做生产换肤与清理。
- 本文档不展开 Phase 2 任务。

### Phase 3 — 收尾与扩展（可选后续）

- `/dev` chunk 构建期剔除。
- 更多语言词条、按需的语言协商。
- 流式打字机节奏等表现层增强（参考 `2026-08-09-frontend-elegance-reference.md`，非本计划范围）。

## 9. 开放问题

已全部在 §0.2 拍板，无阻塞项。遗留可选：

1. **生产包内残留 `/dev` chunk**：门禁后无入口但代码仍在包内（§4 P4 已知限制），可接受；构建期剔除见 Phase 3。
2. **默认 locale 构建期覆盖**：若 Desktop 发行需要英文默认，用 `ROVE_DEFAULT_LOCALE` 在构建时覆盖即可，无需改代码结构。
