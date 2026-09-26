# 会话体验对齐：工具呈现、回合、等待与输入

> 状态：**Implemented（本轮已完成，偏差见 §21 实施记录）**。
> 日期：2026-09-23（实施完成于 2026-09-26）。
> 基线：worktree `fix/ui-shell-layout` @ `d6d6614`（相对 `origin/main` `f853a97` 的外壳修复已在该分支落地，见
> [三栏外壳修复](2026-09-22-ui-shell-layout-remediation.md)）。本文所有"现状"均以该分支为准。
> 参照实现（只读，不引入依赖）。两个项目都在本机，实施时直接读源码，不依赖网络：
>
> - PI-Desktop：`D:\Study\project\agent\third-party-agents\PI-Desktop`
> - open-vetta：`D:\Study\project\agent\third-party-agents\open-vetta`

## 0. 范围与不做的事

上一轮把外壳几何对齐了（宽度预算、动态页签、阅读列、跟随滚动、小地图、命令面板、窄屏悬停、二次确认）。本轮对齐的是**会话内部的体验**：一次回答怎么折叠、工具结果怎么读、没有 token 的等待怎么解释、输入怎么不丢。

硬性边界：

1. **默认不改运行时契约。** 不新增事件类型、不改 `StreamEvent`、不改工具结果信封。W1–W11 都是 `apps/web` 内的投影与呈现。两处例外单独成节：W5 的"停止即回填"只复用既有的 `revoke`；W12 的"编辑重发"需要运行时提供按消息截断的能力，该节写明了在此之前的降级做法。
2. **不持久化布局与阅读偏好之外的新值。** 本轮新增的本机偏好（会话置顶、字号倍率、快捷键覆盖）走 `localStorage`，与既有的 `rove.ui-*` 键一致，不进产品偏好契约。
3. **不照搬外观。** 借鉴的是机制（纯函数、状态分离、预算分层），视觉沿用 v2 皮肤与 `styles/v3` token。
4. **文案进 `copy/zh-CN.ts` 与 `copy/en-US.ts`**，组件内不写死自然语言句子。

明确不做（各自的理由在对应小节）：会话行拖拽排序、多选批量操作、侧栏虚拟滚动、命令使用频率排序、货币成本展示、密度设置、把 `animation-duration: 0.01ms` 代替 `animation: none`、插件市场 / 同步 / 移动端。

## 1. 现状与可直接使用的数据

这些是本轮的输入，实施时不要重新发明：

| 数据 | 位置 | 本轮用途 |
|---|---|---|
| `ToolCallView`（`args` / `output` / `error` / `mutations` / `metadata` / `outcome` / `artifacts`） | `apps/web/lib/rove-state.ts:40` | W1 的呈现输入 |
| `ToolOutputEnvelope`（`content_blocks` / `structured_content` / `artifacts` / `mutations` / `diagnostics`） | `apps/web/lib/rove-types.ts:480` | W1 的富结果 |
| `ToolContentBlock`（`text` / `image` / `audio` / `resource_link` / `embedded_resource` / `unknown`） | `apps/web/lib/rove-types.ts:424` | W1 的分块 |
| `ToolMutation`（`path` / `operation` / `diff`） | `apps/web/lib/rove-types.ts:492` | W1 的 diff |
| `TranscriptRunGroup`（一个 run 的有序 `message` / `tool` / `input`） | `apps/web/lib/rove-state.ts:116` | W2 的分组输入 |
| `Usage`（含 `cached_tokens`）、`ExecutionBudgetUsage`（含 `cost_microunits`）、`ExecutionBudgetLimits.max_total_tokens` | `apps/web/lib/rove-types.ts:3`、`:122`、`:150` | W4 的占用环 |
| `ProductModelDescriptor.context_window`、`ProductSessionRunModelView.context_window` 与 `per_mtok_*` | `apps/web/product/product-api-types.ts:366`、`:380` | W4 的分母与既有计价 |
| `StreamEvent`：`model_status`、`prompt_compacted`、`prompt_built`、`execution_degraded`、`execution_budget_updated`、`plan_step_started`、`finalization_started` | `apps/web/lib/rove-types.ts:535` | W4 的等待阶段 |
| `PromptCompactionState`（`degraded` / `consecutive_failures` / `circuit_open` / `last_error`） | `apps/web/lib/rove-types.ts:321` | W4 的压缩态 |
| `ProductMessage`（`queued` / `intervention_requested` / `applied_current_run` / `claimed_successor` / `needs_attention` / `revoked`，`requested_delivery` 为 `successor` \| `current_run`） | `apps/web/product/product-api-types.ts:1029` | W3、W5 |
| `ProductMessagesResponse.next_before_seq` | `apps/web/product/product-api-types.ts:1057` | W6 的服务端分页 |
| `Transcript` 内 `RUN_PAGE_SIZE = 16` 与 `hiddenRunCount` | `apps/web/chat/Transcript.tsx:33`、`:85` | W6 要替换的客户端窗口 |
| `KEYBOARD_SHORTCUTS`（5 个动作，含 `open-command-palette`） | `apps/web/settings/keyboard-settings-model.ts:35` | W8 的键位数据 |
| `SETTINGS_SECTIONS`（9 个分区，`advanced` 只是路由别名） | `apps/web/settings/sections.ts:1` | W8 的深链目标 |
| `createComposerDraftStore` | `apps/web/state/composer-draft-store.ts` | W5 的草稿落点 |

三处必须先承认的缺口，否则计划会写成"纯呈现"而实际做不到：

1. **没有父子工具关联。** `ToolCallView` 与 `tool_call_started` 都没有 `parent_tool_call_id`。子代理挂靠因此**不做**（§3.4）。
2. **没有思考（thinking）通道。** 事件流里没有独立的思考块，W2 只折叠工具与状态，不发明思考气泡。
3. **`model_status.status` 是自由字符串**，不是枚举。等待阶段必须按已知字符串映射，未知值走兜底（§6.2）。

## 2. 总原则

从两个参考实现里抽出、并要求本轮遵守的五条：

1. **投影是纯函数。** 分组、呈现、阶段归约、占用计算、置顶排序、窗口归约全部写成无 React 依赖的模块，配套单测。组件只负责渲染与交互。参照 `buildToolPresentation`、`buildTranscriptEntries`、`reduceTranscriptWindow` 的形态。
2. **折叠时不算重活。** diff 切行、代码分块、大段格式化只在卡片展开时发生。参照 PI"展开时才生成 `ToolBlock`"。
3. **自动行为与用户接管分开存。** 运行中自动展开、结束后自动收起，但用户点过一次就以用户为准，直到该条目的身份变化。参照 `useAutomaticDisclosure`。
4. **两层预算不合并。** "服务端已加载多少"和"DOM 上挂了多少"是两个数。参照 `reduceTranscriptWindow` 与服务端分页游标分离。
5. **异步结果不挪动用户脚下的东西。** 选中项按 id 锚定、后发请求作废先发、固定组序不按相关性重排。这条上一轮命令面板已经遵守，本轮的搜索、占用环、阶段指示继续遵守。

## 3. W1 工具结果的类型化呈现

### 3.1 现在的问题

`ToolCard`（`apps/web/chat/Transcript.tsx:485`）展开后是三段 `<pre>`（Invocation / Result / Failure）加一份事实表，状态直接渲染原始字符串 `"running"`。`mutations[].diff` 虽已走 `DiffView`，但读文件、列目录、搜索这类结果仍是整段文本。一次展开把全部格式化都做了，无论用户看不看。

### 3.2 做法

新增 `apps/web/chat/tool-presentation.ts`，导出：

```ts
type ToolBlock =
  | { kind: "fields"; rows: { label: string; value: string }[] }
  | { kind: "code"; language: string; text: string; truncated: boolean }
  | { kind: "diff"; path: string; operation: ToolMutationOperation; text: string; truncated: boolean }
  | { kind: "files"; entries: { path: string; annotation?: string }[]; truncated: boolean }
  | { kind: "matches"; entries: { path: string; line?: number; text: string }[]; truncated: boolean }
  | { kind: "note"; text: string };

interface ToolPresentation {
  title: string;        // 工具名
  subtitle: string;     // 一行摘要：读了什么 / 改了什么 / 退出码
  chips: { label: string; tone: "neutral" | "ok" | "warn" | "danger" }[];
  blocks: ToolBlock[];  // 仅展开时计算
}
```

`buildToolPresentation(tool: ToolCallView, expanded: boolean): ToolPresentation`。`expanded === false` 时 `blocks` 为空数组，只算 title / subtitle / chips。

生成规则，按优先级：

1. `mutations` 非空 → 每个 mutation 一个 `diff` 块。**上下文裁剪**：以 `@@` hunk 头为界，每个 hunk 只保留变更行前后各 2 行上下文；总行数超过 400 截断并置 `truncated`，尾部加一行 `note` 说明省略了多少行。这是 PI `buildDiffLines` 的语义。
2. `outcome` 或 `metadata.diff_summary` 表明是搜索类工具 → `matches` 块，上限 200 条。
3. `metadata.affected_paths` 非空且无 diff → `files` 块。
4. `output` 形如代码（以已知语言标记开头，或单文件读回）→ `code` 块，上限 2000 行，带 `truncated`。
5. 其余 → `fields`（从 `metadata` 取 status / risk / read_only / error_code）加一个 `note` 承载 `output` 的前 4000 字符。
6. **永不渲染的字段**：`metadata` 里的原始 id、`protocol_metadata`、任何键名含 `token`、`secret`、`authorization` 的条目。未知工具走第 5 条兜底，不报错。

chips 来源：`metadata.status`、`metadata.risk_level === "high"`、`read_only`、`workspace_changed`、`error_code`、截断标记。状态文字走 copy key，不再直接渲染 `"running"`。

### 3.3 组件

`ToolCard` 改为消费 `ToolPresentation`。头部保留 `aria-expanded` 与 `aria-controls`；详情区按 `blocks` 渲染，`code` 与 `diff` 复用现有 `DiffView` 与 `<pre>`，不引入语法高亮库（见 §3.4）。每个块的"已截断"提供一个按钮展开全文，全文仍受 4000 行上限保护，超出部分只给"在变更页签查看"。

### 3.4 有意不做

- **子代理挂靠**。PI 用 `parentToolCallId` 把子代理收到对应 `Task` 卡片下。我们的事件没有这个字段，硬从时序推断会把无关工具收进别人的卡片。等运行时给出稳定的父子 id 再做，单独立项。
- **增量语法高亮**。PI 的 `tokenizeIncremental` 依赖 Shiki 与 `GrammarState`。引入高亮器是新依赖，违反"不新增依赖"的仓库规则，且本轮的阅读收益主要在 diff 与结构，不在着色。代码块先保持纯文本加语言标签与复制按钮。

### 3.5 测试

`tool-presentation.test.ts`：diff 上下文裁剪与 400 行截断、搜索结果归入 `matches`、`expanded: false` 时 `blocks` 为空、未知工具走兜底、敏感键被丢弃、chips 的 tone 映射。组件层只测"折叠时详情区不在 DOM 里"一条，避免与单测重复。

## 4. W2 回合折叠与阅读锚点

### 4.1 现在的问题

`TranscriptRunGroup.items` 把一个 run 里的 message、tool、input 逐条平铺。一次回答若调用了六个工具，就是六个卡片加若干气泡，用户要滚很远才能看到结论。`ToolCard` 的展开态是 `useState(running || error)`：组件重挂载即丢失"用户手动折叠过"这个事实。

### 4.2 分组

新增 `apps/web/chat/transcript-entries.ts`：

```ts
type TranscriptEntry =
  | { kind: "turn"; id: string; role: "user"; message: ChatMessage }
  | { kind: "activity"; id: string; tools: ToolCallView[]; inputs: TranscriptInputView[] }
  | { kind: "answer"; id: string; message: ChatMessage };
```

`buildTranscriptEntries(items: TranscriptTimelineItem[]): TranscriptEntry[]`，规则：

- 用户 `message` 单独成 `turn`。
- **连续**的 `tool` 与 `input` 合并为一个 `activity`，直到遇到下一条 `message`。
- assistant `message` 成 `answer`。若它是该 run 的最后一条且 `status === "streaming"`，标记为进行中。
- 继承来的父 run（`entry.inherited === true`）照同样规则折叠，不特殊处理。

id 取该组第一条 `entry.id`，保证跨渲染稳定。

### 4.3 自动展开与接管

新增 `apps/web/chat/disclosure.ts`：

```ts
interface DisclosureState { open: boolean; userControlled: boolean }

function reduceDisclosure(
  state: DisclosureState,
  event: { type: "auto"; wantOpen: boolean } | { type: "user"; open: boolean },
): DisclosureState
```

- `auto`：仅当 `userControlled === false` 时采纳 `wantOpen`。
- `user`：置 `userControlled = true` 并采纳。
- `wantOpen` 的来源：组内存在 `status` 为 `running` 或 `waiting` 的工具，或存在 `waiting` 的 input。全部终态后 `wantOpen = false`。

状态按 `activity.id` 存在 `Transcript` 的一个 `Map` 里，而不是卡片内部的 `useState`，这样虚拟化或条件渲染造成的卸载不会丢接管状态。run 被替换（id 消失）时清掉对应项。

### 4.4 阅读锚点

参照 `resolveDisclosureAnchor`。展开或收起前记录被点卡片**标题元素**相对 `.chat-transcript` 视口顶边的偏移；高度变化后（`ResizeObserver` 已有，见 `use-follow-scroll.ts`）把 `scrollTop` 调回该偏移。只在 `pinned === false` 时做——钉在底部跟随最新内容时，高度变化本来就该落到新底，不该钉住旧标题。

实现上挂在 `use-follow-scroll.ts` 旁，新增一个 `holdAnchor(offset)` / `releaseAnchor()`，不改跟随状态机本身。

### 4.5 测试

`transcript-entries.test.ts`：连续工具合并、工具与 input 混合合并、用户消息打断合并、流式尾部标记、继承 run 同规则。`disclosure.test.ts`：自动打开、自动关闭、用户接管后忽略 auto、id 变化重置。e2e 一条：展开卡片后标题的 `getBoundingClientRect().top` 变化不超过 2px。

## 5. W3 排队消息的就地编辑

### 5.1 现在的问题

排队消息已能提升与撤销（`Transcript.tsx` 的 `promotable` / `revocable`），但队列本身是死的：不能改文字、不能调顺序。用户发现写错只能撤销再重发，而重发会排到队尾。

### 5.2 做法

只动 `status === "queued"` 且 `requested_delivery === "successor"` 的消息。`current_run`（插话）不进这个交互，它的语义是"尽快注入当前回合"，允许改序会让注入顺序变得不可解释。

- **编辑**：把该条内容放回 composer（复用 `createComposerDraftStore`），原条保持 `queued` 但置一个本地 `editing` 标记；提交时走既有的"撤销 + 重新创建"，不新增接口。提交失败则恢复原条、草稿留在输入框。
- **调序**：本轮只做**相邻交换**，通过"撤销 + 按新顺序重建"实现。因为服务端队列顺序由 `seq` 决定、没有重排接口，连续重建会有可见的闪动，所以限定一次只移动一位，并在移动期间禁用该条的其他操作。
- **锁定**：`intervention_requested`、`claimed_successor`、`applied_current_run` 不再可编辑或移动，只显示状态。这对应 PI `promoteQueuedPrompt` 的"提升后锁定"，只是我们的锁定由服务端状态驱动，不由本地标记驱动。

### 5.3 风险与回退

"撤销 + 重建"不是原子操作：撤销成功而重建失败时，消息会丢。处理是重建失败时把内容回填到 composer 并给出 `needs_attention` 同级的提示，不静默丢失。若实施时发现该失败窗口在真实网络下频繁出现，本项整体降级为"只能编辑、不能调序"，调序留到运行时提供重排接口之后。

### 5.4 测试

单测覆盖锁定判定与"一次只动一位"。e2e 用现有 `product-api-mock`：编辑后内容变化、移动后两条 `seq` 互换、重建失败时内容回到输入框。

## 6. W4 等待阶段与上下文占用

这是本轮里"数据已在、只差呈现"最集中的一项，也是用户每次对话都能看见的差距。

### 6.1 现在的问题

`WorkbenchState.statusText` 是单字符串（`apps/web/lib/rove-state.ts:130`），来源混着"Connecting to active run"、"Streaming run events"、"Run interrupted"。压缩、重试、降级这些运行时里真实发生的事（`prompt_compacted`、`execution_degraded`、`model_status`）到界面上只剩一个 spinner。`Usage` 与 `context_window` 已在类型里，界面没有占用指示。

### 6.2 等待阶段

新增 `apps/web/chat/activity-phase.ts`：

```ts
type ActivityPhase =
  | { kind: "idle" }
  | { kind: "sending" }
  | { kind: "waiting-model" }
  | { kind: "compacting"; degraded: boolean }
  | { kind: "tool"; name: string }
  | { kind: "planning" }
  | { kind: "finalizing" }
  | { kind: "degraded"; summary: string }
  | { kind: "retrying"; attempt: number };
```

`reduceActivityPhase(phase, event)` 是纯归约，输入为 `StreamEvent` 加上本地的"已发送未确认"。映射：

| 事件 | 阶段 |
|---|---|
| 本地发送、尚未收到 `run_started` | `sending` |
| `run_started`、`llm_chunk` 前 | `waiting-model` |
| `model_status.status` 属于已知集合 | 按表映射，未知字符串 → `waiting-model`，原文放进可展开详情，不直接当标题 |
| `prompt_compacted` | `compacting`，`degraded` 取 `PromptCompactionState.degraded \|\| circuit_open` |
| `tool_call_started` | `tool` |
| `plan_step_started` | `planning` |
| `finalization_started` | `finalizing` |
| `execution_degraded` | `degraded`，文案用 `safe_summary`（若该字段存在）否则用通用句 |
| `llm_chunk` / `run_completed` | `idle` |

**不发明倒计时。** PI 的重试倒计时来自事件里的 `retryAt`，我们的 `model_status` 没有时间字段。没有时间就不画倒计时，避免一个假的进度。

> 后续事实更新（不改写本条决策，实施当时的结论仍然成立）：运行时后来落地了
> R2c 的 `ProviderRetry` 事件，它携带 `attempt`、`max_attempts`、`delay_ms` 和
> 白名单 `reason`（见 `docs/design/2026-09-26-runtime-contract-alignment-design.md`
> §2.4 与 `docs/runtime/react-loop.md`）。也就是说"没有时间字段"这一前提不再成立，
> 但"不做倒计时"仍是前端设计文档 F12 的决策点，默认不做；R2c 只接线了最小投影
> （状态行文案 + 阶段回到 `waiting-model`）。若将来做倒计时，事实来源必须是
> `delay_ms`，不得由前端估算。

呈现：composer 上方一行，`role="status"`、`aria-live="polite"`，只在非 `idle` 时出现。文案全部走 copy key。同一阶段的重复事件不改变文案，避免闪烁。

### 6.3 占用环

新增 `apps/web/chat/context-usage.ts`：

```ts
interface ContextUsage {
  used: number;          // 最近一条带 usage 的消息的 prompt_tokens + completion_tokens
  window: number | null; // 来自 ProductSessionRunModelView.context_window
  cached: number;        // cached_tokens，缺省 0
  ratio: number | null;  // window 为空时为 null
  tone: "neutral" | "warn" | "danger";
}
```

- `tone`：`ratio === null` 为 `neutral`；剩余 ≤ 25% 为 `warn`；≤ 10% 为 `danger`。阈值照 PI，写死为具名常量。
- **窗口未知时不画环**，只显示"已用 N"，不估算一个假的分母。
- 数字可在"已用 / 剩余"间切换，选择存 `localStorage` 键 `rove.ui-context-usage-mode`。
- 缓存命中率 = `cached / (prompt_tokens + cached)`，分母为 0 时不显示。**不显示货币成本**：虽然 `per_mtok_*` 与 `cost_microunits` 已在类型里，但计价可用性（`pricing_availability`）不稳定，错的价格比没有价格更糟。成本留在运行页签的既有度量里，不进这个环。
- 流式期间最新消息还没有 `usage` 时，**沿用上一条有 `usage` 的消息**，不把环闪成 0。参照 `latestTurnContextInspector`。

呈现：一个 16px 的环形指示，放在 composer 右侧、发送按钮旁，`aria-label` 给出"已用 X / 共 Y"。不新增面板。

### 6.4 侧栏终态

`WorkspaceTree` 的会话行已有 spinner 与 warning（`session-item__status`）。补一条规则：会话进入终态后，若该会话不是当前打开的，显示一个点；打开该会话即清除。点只表示"最近一次有结果"，不累积计数，不做收件箱。失败用 `--v2` 的 danger token，成功用 neutral，避免成功淹没失败。状态来源是会话列表里已有的状态字段，不另建通知流。

### 6.5 测试

`activity-phase.test.ts`：每个事件的映射、未知 `model_status` 归 `waiting-model`、重复事件不改变阶段、`run_completed` 归 `idle`。`context-usage.test.ts`：无窗口、阈值边界（恰好 25%、恰好 10%）、缓存分母为 0、沿用上一条 usage。e2e 一条：触发 `prompt_compacted`（mock）后状态行出现且不出现倒计时。

## 7. W5 停止即回填

### 7.1 现在的问题

停止按钮（`Composer.tsx` 的 `onCancel`）中断运行后，输入框是空的，刚发出的消息留在转录里。若模型一个字都没回，用户只能手动复制那条消息再重发。

### 7.2 判定

新增 `apps/web/chat/smart-stop.ts`：

```ts
function shouldRestoreOnStop(items: TranscriptTimelineItem[]): boolean
```

返回 `true` 的条件同时满足：

- 最后一条是用户 `message`；
- 它之后不存在任何 assistant `message`、`tool`、`input`；
- 该条来自本次会话而不是继承的父 run（`entry.inherited === false`）。

`current_run`（插话）产生的消息**不回填**：它被注入到一个已经在进行的回合里，删掉它会让回合上下文与服务端不一致。

### 7.3 动作

`shouldRestoreOnStop` 为真时：先把内容写入 composer 草稿，再调用既有的撤销（`revoke`）路径，成功后从本地转录移除该条。撤销失败则草稿保留、转录保留，并提示"未能撤回，草稿已放回输入框"。

为假时：保持今天的行为，只中断流，把进行中的消息标为终止。

顺序是**先写草稿再撤销**。反过来会在撤销成功、写草稿失败时把内容弄丢。

### 7.4 测试

`smart-stop.test.ts`：无回复时为真、已有工具调用时为假、继承消息为假、插话为假。e2e 一条：发送后立即停止，输入框内容与发送前一致、转录中无该条。

## 8. W6 长会话的两层窗口

### 8.1 现在的问题

`Transcript` 用 `visibleRunCount` 与 `RUN_PAGE_SIZE = 16` 在**已加载的 run** 上做窗口，"Load older turns"只是把本地数组再切一片。真正的历史在服务端，`ProductMessagesResponse.next_before_seq` 与 `product-client.ts` 的 `before_seq` 已经支持分页，但转录没有接。结果是：会话一长，要么一次挂太多 DOM，要么"加载更多"加载不到服务端的更老页。这就是 AGENTS.md 里 F.4 的缺口。

### 8.2 两层预算

新增 `apps/web/chat/transcript-window.ts`：

```ts
interface TranscriptWindow {
  mounted: number;   // DOM 上挂的 run 数
  loaded: number;    // 已从服务端取回的 run 数
}

const INITIAL_MOUNTED = 12;
const STEADY_MOUNTED = 48;
const MOUNT_STEP = 16;
```

`reduceTranscriptWindow(window, event)` 处理两个事件：`grow`（用户滚到顶或点"更早"）把 `mounted` 加 `MOUNT_STEP` 但不超过 `loaded`；`loaded`（服务端页返回）增加 `loaded`。当 `grow` 之后 `mounted === loaded` 且游标 `next_before_seq` 存在，才发起下一次 `before_seq` 请求。

要点是**先挂已加载的，再向服务端要**。反过来会让网络延迟直接表现为滚动卡顿。

### 8.3 稳定幕

参照 `reduceTranscriptSettle`。新增历史页到达后，在转录顶部盖一层 `aria-hidden` 的占位，直到滚动容器的 `scrollHeight` 连续 3 帧不变，最多 600ms，然后撤掉并把滚动位置补偿新增的高度（否则用户会被弹走）。超时也撤，宁愿闪一下也不要永久遮住。

这一层与 §4.4 的阅读锚点共用同一个 `ResizeObserver`，不另起观察者。

### 8.4 与小地图的关系

`ConversationMinimap` 现已约定"标记只指向已渲染回合，`hiddenRunCount > 0` 时轨道仍显示"。本项之后 `hiddenRunCount` 拆成两个数：未挂载（已加载）与未加载（服务端）。小地图的"上面还有历史"提示保留，但文案区分"展开已加载的更早回合"与"从服务器加载更早回合"。不改小地图的标记算法。

### 8.5 测试

`transcript-window.test.ts`：`grow` 不超过 `loaded`、触顶才发请求、游标耗尽后不再发。e2e 用 mock 造 40 个 run：首屏 DOM 中的 run 数 ≤ `INITIAL_MOUNTED`，滚到顶后出现下一次 `before_seq` 请求，且视口中的第一条消息在加载后仍可见。

## 9. W7 侧栏：置顶、搜索、点选先画

### 9.1 会话置顶

工作区已有 `pinned`（`WorkspaceTree.tsx:323`），会话没有。新增本机偏好，键 `rove.ui-pinned-sessions`，值为 `{ sessionId: string; pinnedAt: number }[]`。

新增 `apps/web/sidebar/session-order.ts`：

```ts
function orderSessions(sessions, pinned, now): OrderedSession[]
```

- 置顶在前，按 `pinnedAt` 倒序；其余按既有的修改时间倒序。
- 同一毫秒内重复置顶时，后来者用 `max(pinnedAt) + 1`，保证顺序确定。参照 open-vetta `latestPin + 1`。
- **折叠不能藏置顶**：侧栏收起数量时，可见条数至少等于置顶数。
- 当前打开的会话若落在可见区之外，自动展开到包含它。参照 `useDefaultSessionListModel`。
- 会话被删除时同步从该键里移除，不留悬空 id。

置顶不进 ProductStore：它是本机的阅读习惯，换机器不保留是可接受的，也避免与服务端会话状态互相覆盖。

### 9.2 会话搜索

会话数量上来之后靠滚动找不了。新增一个输入框，规则：

- 180ms 防抖；
- 每次输入递增一个序号，回调里序号不匹配就丢弃（后发作废先发）；
- 只做**子串、大小写不敏感**匹配，与命令面板同一套理由：中文没有词边界，模糊子序列会让"搜不到"不可信；
- 结果为空时给出空态，不回退成全量列表（回退会让用户以为搜到了全部）；
- 查询为空时立即恢复全量，不等防抖。

本轮不做服务端全文搜索。会话标题与 id 都在已加载列表里，本地过滤足够。若列表本身是分页的，输入框下方注明"只搜索已加载的会话"。

### 9.3 点选先画

参照 `selectAfterPaint`。点击会话行时：

1. 同步提交"选中"状态（让高亮先画出来）；
2. 等两帧（`requestAnimationFrame` 嵌套）后再触发会话内容加载；
3. 加载请求带自增 id，后发的响应到达时若 id 不匹配则丢弃。

只包**鼠标点击**。键盘上下移动选中时不做延迟，否则连续按键会堆积一串过期请求。

### 9.4 测试

`session-order.test.ts`：置顶倒序、同毫秒顺序稳定、折叠下置顶仍可见、当前会话强制可见、删除后清理。搜索单测：子串而非子序列、序号作废、空查询立即恢复。e2e 一条：快速连点三个会话，最终只有最后一个的内容出现。

## 10. W8 快捷键数据、字号与设置落点

### 10.1 快捷键覆盖

`keyboard-settings-model.ts` 已有 5 个动作与匹配函数，但覆盖值没有存储、不能解绑、不能迁移。新增 `apps/web/settings/keybindings.ts`：

```ts
type KeybindingOverride = { key: string; modifiers: Modifiers } | null;
// null = 显式解绑
```

- 存储键 `rove.ui-keybindings`，只存**与默认不同**的项。与默认相同的项不写入，方便以后改默认值时用户自动跟上。
- 录键时拒绝：单独的修饰键、与浏览器保留键冲突（Ctrl/Cmd+W、Ctrl/Cmd+N、Ctrl/Cmd+T）、与另一动作冲突。冲突时指出冲突的是哪个动作，不静默覆盖。
- `migrateKeybindings(overrides, table)`：当某个动作 id 被重命名，旧 id 的覆盖折进新 id。现在没有需要迁移的 id，但函数与一条单测先在，避免下次改名时静默丢失用户设置。参照 PI `migrateKeybindingOverrides`。
- 命令面板的 `open-command-palette` 也走这张表，不单独硬编码 Ctrl/Cmd+K。`allowInEditable: true` 的语义保持。

### 10.2 字号倍率

新增 `apps/web/settings/font-scale.ts`：`FONT_SCALE_MIN = 0.85`、`FONT_SCALE_MAX = 1.4`、`FONT_SCALE_STEP = 0.05`、默认 1。存储键 `rove.ui-font-scale`。

应用到 `.product-app-frame` 的一个 CSS 变量 `--font-scale`，v2 的字号 token 改为 `calc(var(--v2-text-*) * var(--font-scale))`。**不逐个组件设字号**，否则漏一处就是一处错位。

范围比 PI 的 0.8–1.5 略收窄：我们的阅读列上限是 840，放得更大行长会破版。步长取 0.05 而不是 0.025，减少无意义的细调。

不做密度设置。两个参考项目都没有单独的密度字段，行距与间距应随字号走，不另开一个维度。

### 10.3 设置深链与落点

命令面板已经能跳到设置分区（`SETTINGS_SECTION_COPY_KEYS`）。补的是**落点反馈**：路由带 `#section` 到达后，目标分区滚动到视口中央，并加上一个 4 秒的高亮类后移除。参照 open-vetta 的 `setting-section-breathe`。

高亮只靠背景色过渡，遵守现有的 reduced-motion：在 `prefers-reduced-motion: reduce` 下直接加最终态、不加过渡。

不建返回栈。页头的返回用浏览器历史即可，与 open-vetta 一致。

### 10.4 测试

`keybindings.test.ts`：相同于默认不存储、`null` 解绑、冲突拒绝、迁移折算。`font-scale.test.ts`：越界钳制、步长对齐。e2e 一条：改字号后 `--font-scale` 生效且刷新后仍在；跳到设置分区后目标在视口内。

## 11. W9 上一轮留下的收尾

这些不是新借鉴，是 [外壳修复文档](2026-09-22-ui-shell-layout-remediation.md) 自己记录、尚未做的事。放在本轮是因为它们小、且与本轮改动的文件重叠。

1. **`--work-panel-collapsed-width` 悬空**（§7.4）。全仓只有 `ProductApp.tsx` 一处引用、无定义，恒取 40px 兜底。改为引用一个具名常量，与 `.product-inspector[data-collapsed="true"]` 的 40px 对齐，删掉这个未定义变量。
2. **右栏拖拽仍逐帧 `setState`**（§8.3 第 1 条）。左栏已经改成直接写 CSS 变量、松手才提交，脚本时间从 604ms 降到 89ms。右栏的 `use-panel-resize.ts` 用同样的方式：预览写 `--work-panel-track`，`pointerup` 才提交 `panelWidth`。先复测再改，不预设收益数字。
3. **`frame-batcher` 与 `latest-wins` 的去留**。两者目前产品代码零调用（`latest-wins` 只接到浏览器里渲染不到的桌面面板）。本轮不新增它们的调用点。若 W4、W6 结束后仍无调用，删掉这两个模块与其单测，不留死代码。
4. **生产构建门禁**（§8.5）。全量 e2e 只在 dev 跑，生产构建下暴露过重复的 `robots` meta 与 `/dev/*` 预览页用例失败。本轮给 `pnpm test:e2e` 增一个生产档入口（复用已有的 `scripts/run-e2e-prod.mjs` 与 `ROVE_E2E_PROD`），并修掉重复的 `robots`。`/dev/workbench` 的失败先确认是否在本轮基线上仍存在，再决定修还是标为已知缺口，不在未复现时就改。

## 12. W10 输入触发：斜杠命令与文件提及

参照 PI `packages/shared/src/composer-trigger.ts` 的 `detectTrigger` 与 `apps/desktop/src/hooks/use-composer-autocomplete.ts`。open-vetta 的 `editor/tokens/trigger.ts` 用另一套规则（允许文字后触发），本项取 PI 的更严版本，理由见下。

### 12.1 现在的问题

`Composer` 是一个纯文本框（`apps/web/chat/Composer.tsx`）。常用动作（新建会话、打开设置、切模型）要么靠记忆快捷键，要么离开输入框去点。引用工作区文件只能手打路径，打错了运行时才会报。

### 12.2 触发规则

新增 `apps/web/chat/composer-trigger.ts`，纯函数：

```ts
type Trigger =
  | { kind: "slash"; query: string; range: [number, number] }
  | { kind: "file"; query: string; range: [number, number]; quoted: boolean }
  | null;

function detectTrigger(text: string, caret: number): Trigger
```

- **斜杠只在草稿第一个非空白 token 内生效**，且光标还在该 token 里。`请帮我看 /src/main.rs` 这种正文里的斜杠不触发。PI 与 open-vetta 在这里分叉，取更严的一个：误触发一次比少触发一次更打乱输入。
- **`@` 要求前一个字符是开头、空白、引号或 `=`**。`a@b.com` 不触发。`@"...` 允许空格，直到闭合引号。
- **IME 组合期间冻结**。`compositionstart` 到 `compositionend` 之间不重新检测、不改菜单内容，避免中文输入时菜单跟着拼音跳。
- 空草稿下，中文顿号 `、` 不改写成 `/`。那是 PI 的桌面特例，我们的输入法行为不一致，不做。

### 12.3 斜杠命令的来源

命令不新造一套注册表，直接复用已有的两个来源：

- `KEYBOARD_SHORTCUTS` 里那些"对当前界面有意义"的动作（新建会话、打开设置、切换检查器），接受后执行动作并清空触发词；
- 命令面板已有的设置分区与主题切换。

不把工作区、会话列表塞进斜杠菜单——那是命令面板的职责，两个入口堆同样的内容会让人不知道用哪个。菜单上限 8 条，匹配沿用命令面板的子串规则。

### 12.4 文件提及的来源

rove 没有 PI 那种 `git ls-files` 索引（`FS_INDEX_MAX_ENTRIES = 8000`）。本项用**已打开工作区的文件列举**，上限 200 条，超出时菜单底部注明"只显示前 200 个，输入更多字符缩小范围"。不建索引、不跑 git。

接受后的插入文本：

- 文件 → 插入路径加一个空格，路径含空格时用引号包住；
- 目录 → 插入路径加 `/`，**不加空格**，让用户继续补。

这比 PI 的"文件变附件、发送时再序列化"简单。我们没有附件系统（§13 才引入粘贴附件），先把路径当作文本，运行时本来就按路径读文件。

### 12.5 菜单行为

与命令面板同一套键盘：上下移动、Enter 接受、Escape 关闭、Tab 不离开输入框。菜单用 `role="listbox"`，激活项用 `aria-activedescendant`。触发词被删除或光标离开范围时菜单关闭。数据源加载失败时菜单显示空态，不报错、不阻塞输入。

### 12.6 测试

`composer-trigger.test.ts`：斜杠只在首位 token、邮箱不触发、引号内允许空格、光标离开即失效、IME 冻结期间结果不变。e2e 一条：输入 `/` 看到命令、接受后输入框清空并执行；输入 `@` 后选择文件，路径插入到光标处。

## 13. W11 大段粘贴与拖放

### 13.1 现在的问题

把一整段日志贴进输入框，草稿会膨胀到几千字，既难编辑，又会原样进 prompt。拖一个文件进来，浏览器默认是打开或无视，没有"按路径引用"这个选项。

### 13.2 大段粘贴

参照 PI `DEFAULT_LARGE_PASTE_THRESHOLD = 600`（按 Unicode code point 计）。新增 `apps/web/chat/composer-paste.ts`：

- 纯文本超过 600 个 code point：阻止默认粘贴，收成一个附件 chip，内容保存在内存里，随草稿走（不落盘，刷新即丢，与草稿存储同一生命周期）。
- chip 显示前 40 个字符与总长度，可删除，可点击展开查看全文（只读）。
- 发送时把附件内容按围栏代码块附在 prompt 末尾，并在 chip 原位置留一行标记说明"以下为粘贴内容"。
- 600 以下的粘贴保持原样，富文本粘贴只取纯文本，丢掉 HTML。

上限：一次草稿最多 10 个粘贴附件、合计 256 KB。超出时拒绝本次粘贴并在输入框下方说明原因，不截断、不部分接受。数字比 PI 的（20 个、合计 128 MB）小，因为我们把内容放在浏览器内存里而不是主进程落盘。

### 13.3 拖放

参照 `composerDropItems`，但浏览器拿不到可靠的本地路径，所以规则收窄：

- **文本拖入**（选中文本）按粘贴处理，同样受 600 阈值约束。
- **文件拖入**：不读内容。若拖拽数据里带路径（工作区内部拖出的文件，我们自己写的 `text/plain` 路径），插入为 §12.4 的路径引用；否则拒绝并提示"浏览器不能读取该文件的路径，请用 @ 提及"。
- **目录**：一律不读。没有路径就拒绝，理由同上。

不做"把拖入的文件上传为附件"。那需要体积、类型、病毒面的一整套处理，超出本轮。

### 13.4 测试

`composer-paste.test.ts`：599 与 600 的边界、按 code point 而不是 UTF-16 码元计数、超限拒绝、富文本只取纯文本。e2e 一条：粘贴 1000 字后输入框里是 chip 而不是文本，发送请求体里包含这段内容。

## 14. W12 消息操作：复制、重试、编辑、重命名

### 14.1 现在的问题

消息气泡上没有任何操作。复制要手动选择，选多了会带上按钮文字；对一条不满意的回复，只能在下面再发一句"重来"，旧回复还留在上下文里继续影响后续；会话标题靠首条消息自动截断（`use-session-continuity.ts` 的 `truncateTitle`），改不了。

### 14.2 复制与重试（不需要新接口）

操作按钮放在气泡右上，默认隐藏，**气泡 hover、气泡内 focus、或按钮自身聚焦**时显示。参照 PI `.message-actions` 的三条件，第三个条件保证键盘用户用得到。

- **复制**：`navigator.clipboard.writeText` 写该条的原始 markdown，成功后图标变勾，1500ms 后恢复。不区分"复制 markdown"与"复制纯文本"——两个参考实现都没有这个区分，多一个选项就是多一次选择。
- **重试**（只在最后一条 assistant 回复上出现）：把对应的用户消息内容作为**新的一条**发送。不删除旧回复。这是有意的降级：PI 的 `retryAssistantMessage` 会带 `truncateFromMessageId` 把旧回复从上下文里截掉，我们的运行时没有这个能力（`apps/api` 里没有按消息截断的接口）。在它存在之前，重试就是"再问一次"，旧回复保留。

### 14.3 编辑重发（需要运行时，先做降级）

PI 的 `editUserMessage` 用新内容替换该条用户消息，并让服务端截掉它之后的全部历史。这是对的语义，但依赖按消息 id 截断：找不到 id 时返回明确错误，**禁止按序号猜测截断点**。

本轮的降级：编辑只对**最后一条用户消息**开放，把内容放回 composer，不提交时不产生任何服务端变化。提交时按 §14.2 的重试处理（再发一条）。完整的"截断后重发"单独列为运行时需求，不在本文件里设计其协议。

### 14.4 会话重命名

接口已有：`productClient.updateSession(sessionId, { title })`（`apps/web/product/product-client.ts:536`）。侧栏会话行双击标题，或行菜单里的"重命名"，变成一个输入框，Enter 提交、Escape 取消、失焦提交。

- 提交期间锁定该行，失败时恢复旧标题并在行内显示错误，不弹全局 toast。
- 空标题拒绝提交，保持编辑态。
- 标题上限沿用服务端已有的限制，前端不另定一个。

### 14.5 分支

`forkSession` 已存在（`ProductApp.tsx:479`）。本项只把它放到消息操作里：对一条用户消息选择"从这里分叉"，调用既有 fork 并打开新会话。**不保证截断点精确到该条消息**——现有 fork 复制的是会话而不是消息前缀。若 fork 的语义是整会话复制，按钮文案就写"复制会话"，不写"从这里分叉"，避免承诺做不到的事。实施时先核对 `forkSession` 的实际语义再定文案。

### 14.6 测试

e2e：复制后面板读到 clipboard 内容；重试产生一条新的用户消息且旧回复仍在；重命名失败时标题回退；键盘 Tab 能到达隐藏的操作按钮。

## 15. W13 代码块与流式渲染

### 15.1 现在的问题

代码块没有复制按钮、没有语言标签。流式输出时，未闭合的 ` ``` ` 围栏会让后续文本被吃进代码块，等到闭合才跳出来，阅读被打断。

### 15.2 代码块头部

参照 PI `components/Markdown.tsx` 的 `CodeBlock` 与 open-vetta 的 `CodeBlock.tsx`：

- 头部左侧是语言标签，语言为空时显示 `text`，不留空白。
- 右侧是复制按钮，复用 §14.2 的 1500ms 反馈。
- 长行横向滚动，不换行。两个参考实现都没有"切换自动换行"，本项也不加。
- 不引入语法高亮，理由同 §3.4。

### 15.3 流式时的围栏

新增 `apps/web/chat/streaming-blocks.ts`，纯函数，参照 open-vetta `stable-blocks`：

```ts
interface BlockSplit { stable: string; tail: string }
function splitStableBlocks(markdown: string): BlockSplit
```

- `stable` 只包含**已经闭合**的顶层围栏及其之前的文本。
- `tail` 是剩余部分，原样渲染为普通 markdown。
- 未闭合的围栏留在 `tail` 里，按普通文本显示，不升级成代码块。闭合的瞬间它从 `tail` 移到 `stable`。
- 只认顶层围栏。缩进代码块与嵌套围栏不参与冻结，避免误判。

这样做的代价是代码块在闭合前没有高亮背景，收益是流式过程中正文不会被吞。值得。

不做法：按短语淡入（open-vetta `streaming-reveal` 的 50–300ms 间隔、800ms 强制放出）。它需要一个动画调度器，并与 reduced-motion 纠缠，而我们的流式渲染本身是 chunk 驱动的，没有"积压"问题。

### 15.4 测试

`streaming-blocks.test.ts`：未闭合围栏全部留在 tail、闭合后移入 stable、围栏前的文本属于 stable、嵌套与缩进不触发冻结、空字符串。e2e 一条：流式中间态下，围栏之后的文字仍按正文渲染。

## 16. W14 草稿收养与发送历史

### 16.1 现在的问题

`createComposerDraftStore`（`apps/web/state/composer-draft-store.ts`）按会话保存草稿，但新建会话有一个空窗：用户在"还没有会话"的页面打了字，点新建后，这些字留在旧槽里，新会话是空的。发过的消息也无处召回，上箭头不能翻出上一条。

### 16.2 收养

参照 PI `composer-draft-cache.ts` 的 `adoptHomeDraftForSession` 与 open-vetta `session-input-draft.ts` 的 `claimNewSessionInputDraft`。

- 无会话页面的草稿存在一个固定槽 `new:<workspaceId>`。
- 会话创建成功的回调里，把该槽的内容移到新会话的槽，并**删除旧槽**。顺序是先写新槽、再删旧槽，失败时旧槽还在，不丢字。
- 删除必须在下一次按键之前完成。若创建是异步的，期间用户又打的字也要一并搬走，所以搬家发生在"新会话 id 到手"的那一次状态更新里，而不是在点击的瞬间。
- 空槽不搬家，避免把新会话里已有的草稿覆盖成空。

### 16.3 发送历史

参照 open-vetta 的 `INPUT_HISTORY_MAX = 50`。每个会话保留最近 50 条已发送文本，存在内存里，刷新即丢。

- 输入框为空时，上箭头取出上一条，下箭头向前。
- 输入框非空时，上箭头保持浏览器默认行为（移动光标），不劫持。
- 历史只在"发送成功"时记一条。发送失败不记，否则召回的全是没发出去的。

### 16.4 测试

单测：搬家后旧槽为空、新槽为原内容、空槽不覆盖、异步期间追加的文字也被搬走。e2e 一条：在空页面打字、新建会话、文字还在；发送后清空输入框，上箭头召回刚才的文本。


## 17. 明确不做的清单

| 项 | 来源 | 不做的理由 |
|---|---|---|
| 子代理按父工具挂靠 | PI `parentToolCallId` | 事件流没有父子 id，推断不可靠（§3.4） |
| 思考块折叠 | PI thinking | 没有思考通道，不能发明内容 |
| 增量语法高亮 | PI `tokenizeIncremental` | 需要新增 Shiki 依赖 |
| 重试倒计时 | PI `AgentActivity.retrying` | `model_status` 没有时间字段 |
| 货币成本 | — | 计价可用性不稳定；且 PI 自己也不显示 |
| 会话拖拽排序 / 多选 / 批量 | — | open-vetta 也没做，收益低于复杂度 |
| 侧栏虚拟滚动 | — | 参考实现用"默认若干条 + 展开"，我们沿用 |
| 命令使用频率排序 | — | 会破坏上一轮"固定组序"的决策 |
| 密度设置 | — | 两个参考都没有，行距随字号 |
| `animation-duration: 0.01ms` | PI reduced-motion | 全仓无 `animationend` 监听，`animation: none` 不会卡住卸载 |
| 服务端会话全文搜索 | — | 本轮本地子串足够 |
| 队列的原子重排（Web 接线） | — | 原理由“运行时无重排接口”已被 [运行时文档](../design/2026-09-26-runtime-contract-alignment-design.md) R4（2026-09-26）解除：`POST /product/sessions/{session_id}/messages/reorder` 提供单事务原子重排。W3 目前仍是“撤销 + 重建”的相邻交换并带回退；改用该端点的 Web 接线归前端文档，详见该设计 §4.4 |
| 按消息截断后重发 | PI `truncateFromMessageId` | 运行时无此接口；W12 先做"再发一条"，协议另案 |
| 短语淡入的流式揭示 | open-vetta `streaming-reveal` | chunk 驱动无积压，不值得加动画调度（§15.3） |
| 文件上传为附件 | — | 体积、类型与安全处理超出本轮；拖放只接受路径（§13.3） |
| 复制 markdown / 纯文本分项 | — | 两个参考都只做一种复制（§14.2） |
| 代码块自动换行开关 | — | 两个参考都没有，长行横向滚动（§15.2） |
| 中文顿号改写为斜杠 | PI `rewriteIdeographicCommaTrigger` | 桌面输入法特例，Web 行为不一致（§12.2） |

## 18. 实施顺序与依赖

```
W1 工具呈现 ──┐
W2 回合折叠 ──┤（都改 Transcript，串行做，先 W1 后 W2）
W6 两层窗口 ──┘
W13 代码块与流式 ── 改 Transcript 的渲染，排在 W2 之后
W4 等待与占用 ── 独立，可与上面并行
W5 停止回填 ── 依赖 W2 的条目模型，排在 W2 之后
W3 队列编辑 ── 与 W5 共用撤销路径，排在 W5 之后
W10 输入触发 ──┐（都改 Composer，串行做，先 W10 后 W11）
W11 粘贴与拖放 ┘
W14 草稿收养 ── 改草稿存储，排在 W11 之后
W12 消息操作 ── 复制/重试/重命名不依赖运行时；编辑重发的完整版另案
W7 侧栏 ── 独立
W8 键位与字号 ── 独立
W9 收尾 ── 最后
```

建议的落地批次：

- **批次 A**：W1、W2、W4、W13。全是纯投影，风险最低，体感最大。
- **批次 B**：W6、W5、W7、W10、W14。涉及滚动、撤销、侧栏与输入，需要 e2e。
- **批次 C**：W3、W8、W9、W11、W12。W3 有失败窗口，W11 改发送体，W12 含降级决策，单独评审；W9 含删除决策。

## 19. 验收

每批合并前：

- `pnpm --dir apps/web exec vitest run` 覆盖新增单测；
- `pnpm --dir apps/web typecheck`；
- 受影响的 e2e：`transcript-scroll`、`conversation-minimap`、`workbench-panel`、`command-palette`，外加本文各节新增的用例；
- 批次 C 额外跑一次 `ROVE_E2E_PROD=1` 的生产档（§11.4）。

不在本轮跑 Rust 门禁：W1–W11 与 W13、W14 没有 Rust 改动。W12 的完整版（按消息截断）一旦启动就必须先补运行时测试，那属于另一份变更说明，不在本文件范围内。

## 20. 文件清单（预期）

新增：

- `apps/web/chat/tool-presentation.ts` 与测试
- `apps/web/chat/transcript-entries.ts` 与测试
- `apps/web/chat/disclosure.ts` 与测试
- `apps/web/chat/activity-phase.ts` 与测试
- `apps/web/chat/context-usage.ts` 与测试
- `apps/web/chat/smart-stop.ts` 与测试
- `apps/web/chat/transcript-window.ts` 与测试
- `apps/web/sidebar/session-order.ts` 与测试
- `apps/web/settings/keybindings.ts` 与测试
- `apps/web/settings/font-scale.ts` 与测试
- `apps/web/chat/composer-trigger.ts` 与测试
- `apps/web/chat/composer-paste.ts` 与测试
- `apps/web/chat/streaming-blocks.ts` 与测试

修改：

- `apps/web/chat/Transcript.tsx`、`Composer.tsx`
- `apps/web/chat/use-follow-scroll.ts`（锚点挂载点）
- `apps/web/chat/ConversationMinimap.tsx`（文案区分两层历史）
- `apps/web/sidebar/WorkspaceTree.tsx`
- `apps/web/settings/keyboard-settings-model.ts`、`KeyboardSettings.tsx`
- `apps/web/styles/product-v2.css`（`--font-scale`、占用环、阶段行）
- `apps/web/copy/zh-CN.ts`、`en-US.ts`
- `apps/web/shell/ProductApp.tsx`（移除悬空变量、右栏拖拽提交点）
- `apps/web/state/composer-draft-store.ts`（草稿收养与发送历史）
- `apps/web/state/use-session-continuity.ts`（重命名失败回退）

可能删除：`apps/web/lib/frame-batcher.ts`、`apps/web/lib/latest-wins.ts` 及两者的测试（见 §11.3，有条件）。

## 21. 实施记录（2026-09-26）

W1–W13 与 W14 的发送历史已实现并验证；以下记录与原计划的偏差和未接线项。每条偏差都保留原计划的意图，不改变运行时契约。

**W2 回合折叠**：除计划内容外，**阻塞型交互（待审批工具、等待输入）永远渲染在折叠体之外**——接管状态属于读者，但被审批卡住的 run 不能因此失去可交互入口。单测覆盖（`Transcript.test.tsx`）。

**W4 侧栏终态**：只实现了**失败点**（`status === "error"` 且非当前打开 → danger 点）。成功点做不出来：会话状态字段里"刚完成的 idle"与"从未运行的 idle"不可区分，而计划禁止另建通知流。等待运行时提供"最近一次结果"字段后再补。

**W4 阶段归约挂载点**：`reduceActivityPhase` 由 `use-session-continuity` 的 dispatch 包装层喂入——所有 reducer 事实（流事件、job 同步、reset、busy 翻转）都经过这一条路，phase 不会看到 reducer 没看到的东西。`model_status` 事件实际字段是 `status`（计划写对了）。

**W6 两层窗口**：`transcript-window.ts` 与挂载预算已接线；**服务端分页未接线**——transcript 端点（`read_transcript`）只收 `session_id`，没有游标；`before_seq` 分页只存在于消息台账端点，那是另一个投影，不能混用。`shouldRequestOlderPage` 的决策点保留在模块里并有单测，等 API 提供 transcript 游标后即可接上。这是需要单独立项的运行时缺口。

**W7 侧栏**：置顶（`rove.ui-pinned-sessions` + `orderSessions`/`prunePins`）、重命名（双击标题或行内铅笔，Enter 提交 / Escape 取消 / 失焦提交，空标题拒绝，失败恢复）、点选先画（flushSync 语义简化为同步 setState + 双 rAF 提交导航，序号作废后发，键盘激活立即执行不设延迟——参照 open-vetta `useSidebarSelectionIntent` 与 `waitForCommittedPaint`）、会话搜索（上一轮已写好的 `session-search.ts` 纯函数本轮接入 WorkspaceTree，语义与原内联过滤一致：子串、大小写不敏感、空查询立即恢复）均已实现。**一处条件项未做**：侧栏本无"收起数量"UI，`visibleWhenCollapsed` 的接线等该 UI 出现后再接；本地同步过滤不需要防抖与序号作废，这两个机制保留在模块与单测里供异步搜索使用。

**W8**：键位覆盖（存储 `rove.ui-keybindings`、录键、与浏览器保留键和其它动作冲突时拒绝并指名冲突项、解绑、恢复默认、`migrateOverrides` 折叠点、`open-command-palette` 同表解析）已实现；`ShortcutBinding` 增加了 `primary` 字段——没有它无法区分 `Ctrl+K` 与裸 `K`，这是原模块的设计缺口。字号倍率（0.85–1.4、步长 0.05、`rove.ui-font-scale`、GeneralSettings 控件、框架基础字号与 rich-text 标题走 `calc(px * var(--font-scale))`）已实现；11px 的辅助文本有意不缩放。设置呼吸高亮已实现（900ms×4≈3.6s，reduced-motion 直接给终态）；**落点滚动用 `block: "start"` 而非计划的"视口中央"**——设置面板通常高于视口，居中会把标题推出屏幕。

**W9 收尾**：悬空变量 `--work-panel-collapsed-width` 改为具名常量 40px；右栏拖拽改为**预览直写 `--work-panel-preview` CSS 变量、`pointerup` 才提交 `panelWidth`**，不再逐帧 setState（原先 v2 轨道根本不消费 dragWidth，每帧渲染纯属浪费）；ARIA 值改为报告已提交宽度。`frame-batcher` 零调用方，模块与测试已删除；**`latest-wins` 保留**——它有真实生产调用方（SettingsShell 的桌面服务配置探测门），原计划"零调用"的前提已过时。生产档 e2e 入口（`test:e2e:prod` + `ROVE_E2E_PROD`）与 `scripts/run-e2e-prod.mjs` 在本轮之前已存在，无需新增。**重复 robots 未复现**：源码中只有 `app/dev/product-ui-v2/page.tsx` 一处 robots 声明，按"不在未复现时就改"的纪律不动。

**W12 消息操作**：复制、重试（对最后一条 assistant 回复，把对应提问作为**新消息**重发——运行时没有按消息截断的能力，这是计划内降级）、编辑（只对最后一条用户消息，内容回填 composer，提交即重发）、复制会话（消息操作里只暴露会话级 fork——`createFork` 锚定在最近一个已完成 run，不保证截到所选消息，按钮文案按计划要求如实写"复制会话"）、侧栏重命名均已实现。

**W14 草稿收养（§16.2）不适用**：本产品壳在无会话时渲染 `WorkspaceSessionEmpty`，**没有输入框**，"在还没有会话的页面打字"这个场景不存在，草稿存储只挂真实会话 id。发送历史（§16.3，50 条、仅成功发送记录、空框上箭头召回）已实现。

**验证**：`pnpm exec vitest run` 511 个单测通过（含 transcript-window、font-scale、keybindings 新增断言与改写后的 Transcript 折叠断言）；`pnpm typecheck` 通过；`pnpm build` 生产构建通过；全量 e2e（含 `tests/e2e/ui-visual.spec.ts` 视觉冒烟）116 通过、7 跳过（opt-in 真实服务用例）、0 失败；全部 22 张关键界面状态截图逐张人工检查通过。Rust 门禁不适用：本轮无 `apps/web` 之外的改动。

**视觉验收修正的问题**（`ui-visual.spec.ts` 的截图巡检抓到，均已修复）：

1. **disclosure 自动同步的无限重渲染**：`reduceDisclosure` 对自动事件恒返回新对象，效应里按引用比较导致每帧 setState。改为按值比较。单测是 SSR 渲染、不跑效应，只有真实浏览器暴露。
2. **消息复制按钮误用代码块文案**（「复制代码」→「复制」）：新增 `chat.messageCopy`/`messageCopied`。
3. **文件提及菜单的空态永不显示**：菜单可见性此前依赖"有结果"，结果为空时整个菜单消失，违反 §12.5 的空态要求。改为 file 触发期间菜单常显（空态提示在菜单内），slash 无匹配仍收起。
4. **e2e mock 的批准事件双写**：`segment.events` 与 `job.events` 是同一数组的两个别名，批准 route 对两者各 push 一次，完成事件重复、seq 跳号，恢复解析器按设计正确拒绝（`空历史不会被伪造`），表现为批准→完成后转录清空。mock 只 push 一次。这是测试基建的预存在缺陷，被本轮的重恢复链路放大暴露。
5. **点选先画与测试的竞态**：两帧屏障内 fill 落在旧会话的草稿槽。屏障语义与 open-vetta 一致（屏障期间输入属于仍挂载的会话），并改用参考实现的 `flushSync` 提交高亮；相关 e2e 改为等待目标面板出现后再交互。
