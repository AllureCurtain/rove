# Web 前端体验对齐（下一轮）设计

- 日期：2026-09-26
- 状态：**Proposed / Not Implemented**。本文档全部条目均未实现；实施时必须在同一变更中把
  `docs/runtime/implementation-status.md` 与 `docs/runtime/acceptance-matrix.md` 的对应行更新为真实状态。
- 基线：`main @ 915cbb9`（2026-09-26）。
- 范围：仅 `apps/web`（含 `apps/web/tests/`）与仓库级脚本。除「依赖运行时合同」标注外，
  任何条目不得以改 `apps/api` / `runtime/` 合同为实现手段；需要合同的能力一律指向
  [运行时合同对齐（下一轮）设计](2026-09-26-runtime-contract-alignment-design.md)（下称"运行时文档"），
  由那一侧先行落地。
- 参考副本（只读，仅参考交互思路与机制，不复制源码、品牌或素材）：
  - `D:\Study\project\agent\third-party-agents\PI-Desktop`（LGPL-3.0；**禁止**复制其代码，交互思路需独立实现）
  - `D:\Study\project\agent\third-party-agents\open-vetta`（Apache-2.0）
- 前置阅读：`AGENTS.md`、`docs/runtime/README.md`、
  `docs/design/2026-09-17-pi-desktop-workbench-design.md`、
  `docs/plans/2026-09-22-ui-shell-layout-remediation.md`、
  `docs/plans/2026-09-23-conversation-experience-alignment.md`（下称 W 文档；本文档的编号
  延续其 W1–W14 之后的思想，但使用新的 F 前缀）。
- 与本文档配套：`docs/design/2026-09-26-runtime-contract-alignment-design.md`。

---

## 0. 条目总表

| 编号 | 条目 | 优先级 | 依赖 | 状态 |
|---|---|---|---|---|
| F1 | 会话切换的每会话 UI 状态快照（滚动/跟随/窗口/披露） | P0 | 无 | Implemented |
| F2 | 工具与活动耗时呈现（徽标 + 活动组 elapsed） | P0 | 无 | Implemented |
| F3 | 流式 markdown 的分块 memo 化 | P0 | 无 | Implemented |
| F4 | 会话 hover 卡片 | P1 | 无 | Implemented |
| F5 | Toast 通知层与后台失败可见性 | P1 | 无（收件箱部分依赖 R5） | Implemented（toast 层与后台失败；收件箱待 R5） |
| F6 | smart-stop 发送快照（含粘贴 chips） | P1 | 无 | Implemented（部分中止文案分支待 R2b） |
| F7 | 杂项清理（/dev 文案、mock 页标注、降级语义如实标注） | P1 | 无 | Implemented |
| F8 | 样式 token linter 与 CI 接入 | P1 | 无 | Implemented（本地门；按 §9.1 不接入 CI） |
| F9 | 侧栏折叠动效合成器友好化 | P2 | 无 | Implemented（轨道一步到位 + 侧栏滑出；视觉偏差见 §10.4） |
| F10 | 偏好小项（per-model reasoning 记忆、会话自动标题） | P2 | 无 | Implemented（自动标题的前提修正见 §11.3） |
| F11 | 服务端会话搜索 `q` 接线 | P2 | 无 | Implemented（前提复核与差异记录见 §12.1） |
| F12 | 决策点（默认不做）：发送键 morph/涟漪、phrase reveal、跨引擎/视觉回归、系统通知 | — | — | 不做（除非用户拍板） |
| F13 | 附件/图片上传 UI | — | **阻塞于运行时文档 R8** | 不在本轮 |

优先级含义：P0 = 下一轮第一批，必须先做且相互独立可并行；P1 = 第二批；
P2 = 有真实收益但收益/风险比偏低，允许延后。

---

## 1. 现状要点（实施者必读）

以下事实已逐条核实到源码，是本文档所有设计的前提。实施者如果发现这些前提随代码演进而失效，
必须停下来更新本文档，而不是按过期前提实现。

1. **Transcript 切换会话不重挂载**：`shell/ProductApp.tsx:1219` 的 `<Transcript>` 没有
   `key`；而 `RunInspector` 按 `workspaceId:sessionId` 重挂载（`ProductApp.tsx:1296-1297`）。
   切换会话时丢弃的是数据与状态，不是 DOM：`navigateSession` → `prepareSession`
   bump generation、关闭 SSE、清空 controls/messages 并 `dispatch({type:"reset"})`
   （`state/use-product-route-sync.ts:219-224`、`state/use-session-continuity.ts:408-419`）。
2. **切换后强制跳底**：`chat/Transcript.tsx:360-364` 有一个以 `restoreState.sessionId`
   为依赖的 `pinToLatest()` effect——滚动位置在切换时是被主动放弃的。
3. **窗口状态在组件 state**：`chat/transcript-window.ts` 的 `{mounted, loaded}` 存在
   `Transcript.tsx` 的 `useState`（`initialTranscriptWindow`）；新 timeline 比当前 `loaded`
   短时重置（同一文件里的同步 effect）。`shouldRequestOlderPage` 已实现并带单测。
   **更新（2026-09-26）**：服务端 transcript 游标已落地（见
   [运行时与产品合同对齐](2026-09-26-runtime-contract-alignment-design.md) §1），休眠前提失效。
   窗口决策点现由 `olderHistoryControl(window, older)` 表达：`loaded` 里仍有未挂载的 run 时给
   "load older turns"（grow）；全部挂载且服务端 `has_more` 为真时才给
   `button.load-older-history`（server）；游标耗尽后两者都不渲染。prepend 锚定复用
   `prependHeightRef`，失败时保留游标并显示有界错误。
4. **披露状态以 entry id 剪枝**：`Transcript.tsx:263-306`——切换后旧 id 消失，披露 map
   相应清空。entry id 内嵌 run+seq（`lib/rove-state.ts:537-563`），同一 run 稳定。
5. **follow 状态机是 per-instance ref**：`chat/use-follow-scroll.ts:41-46`
   （`pinnedRef`、`lastScrollTopRef`、`lastGestureAtRef`、`frameRef`）。
6. **线上事件不带时间戳**：SSE 帧是 `{ seq, event }`（`apps/api/src/types.rs:165-169`、
   `lib/rove-types.ts:930-933`）；`tool_call_started/completed` 均无 ts
   （`lib/rove-types.ts:615-634`）。唯一现成时长是 `ToolProtocolMetadata.duration_ms?`
   （`lib/rove-types.ts:465`），目前无任何组件读取。
7. **流式渲染无分块 memo**：`chat/streaming-blocks.ts:15-56` 的 `segmentMarkdown`
   把消息切成 prose/code 段（未闭合围栏保持 prose）；`product-v2/RichText.tsx:34-36`
   对每段渲染 `<RichTextMarkdown key={index}>`，但没有 `React.memo`/`useMemo`——
   每个 `llm_chunk` 追加 delta 后，整条消息的所有段都重新走一遍 ReactMarkdown
   （`lib/rove-state.ts:1538-1570` 的 `appendAssistantDelta` 只改最后一个流式消息）。
8. **smart-stop 只回填文本**：`chat/smart-stop.ts:17-42` 判定"模型产出为空"（最后一条
   用户消息之后没有任何非用户条目）；恢复在 `shell/ProductApp.tsx:646-662` 走
   `writeComposerDraft`——draft 快照 `{text, version, submitting, sendFailed}`
   （`state/composer-draft-store.ts:6-11`）不含 chips；chips 在 Composer 本地 state，
   发送时折叠成 `[pasted content: N characters]` 围栏文本（`chat/composer-paste.ts:50-62`）后即丢失。
9. **无 toast/通知系统**：全仓库 grep `toast|snackbar|notification` 仅命中一处注释；
   一切提示都是行内 `role="alert"/"status"` 段落。后台会话失败只有侧栏红点 +
   2.5s 目录轮询（`state/use-server-product-state.ts:353-396`）。
10. **服务端会话搜索存在但未被 Web 使用**：`GET /product/sessions` 支持 `q`
    （title LIKE 子串，128B 上限，`apps/api/src/product/routes.rs:213-218`）；client 也支持
    （`product/product-client.ts:514`），但两个调用方都只传 `cursor/limit/includeArchived`。
    目录当前把会话整页拉全，本地子串过滤（`sidebar/session-search.ts:16-40`）与之等价——
    这是 F11 降为 P2 的原因。
11. **侧栏折叠是 grid 轨道宽度动画**：`.product-body` 的
    `grid-template-columns` 带 transition（`styles/product-v2.css:3646-3657`），折叠把首列
    轨道变 0（`:3784-3791`）同时侧栏 `transform: translateX(-102%)` 滑出；中列在 240ms 内
    连续 reflow。子树保持挂载，aria/inert 处理在 `:3782-3783` 注释中被标记为待办关切。
12. **hover 卡可用数据**：`ProductSession` DTO（`product/product-api-types.ts:117-128`）
    提供 title/status/created_at/updated_at/runtime_binding/fork 血统；模型需按会话懒加载
    model-config 端点（`:348-356`）。
13. **Playwright 只有 chromium 一个 project**（`playwright.config.ts`），mock API 装置在
    `tests/e2e/product-api-mock.ts`（`installMockProductApi`，默认端口 13043），
    生产构建档位为 `ROVE_E2E_PROD === "1"`（`pnpm test:e2e:prod`）。
14. **文案**：新增 UI 文案必须同时加 `copy/zh-CN.ts`（权威）与 `copy/en-US.ts`
    （必须逐键镜像，`copy/en-US.ts:1-2`），用 `t("section.key")`。

---

## 2. F1 会话切换的每会话 UI 状态快照

### 2.1 背景与参考

- open-vetta：为虚拟列表做**每会话滚动快照缓存**（`getState()` 按 session 存 LRU 24，
  恢复时校验 item 身份），切回会话时视口精确还原。
- PI-Desktop：每会话一个 React 窗格、最多保留 3 个，隐藏而非卸载（`visibility:hidden` +
  `content-visibility:hidden`，其 ADR 0137），热切换零骨架、滚动位置原样保留。

### 2.2 方案选择：快照缓存，而非多窗格保留

PI 的多窗格方案要求每个窗格各有一份会话数据与事件订阅。rove 的数据层是
**单 reducer + focused-only SSE**（`use-session-continuity.ts:113` 单 `RunController`；
只有聚焦会话的流事件被应用，`use-session-continuity.ts:262-282`）。引入多窗格等于
把 continuity 重构成多实例 + 多路 SSE——重构风险与收益不成比例。因此本轮采用
vetta 式**状态快照**：DOM 仍单实例，但把"离开会话瞬间"的 UI 状态存下来，回到会话时还原。

### 2.3 目标行为

- 离开会话（`prepareSession` reset 之前）捕获快照：
  `{ scrollTop, followPinned, window: {mounted, loaded}, disclosure: Map 副本 }`。
- 回到会话（restore 完成后）若快照通过**身份校验**则还原：scrollTop 写回滚动容器、
  窗口状态写回、披露 map 用副本替换（而不是被 live id 剪枝清空）；
  `pinToLatest()` effect 改为"无有效快照时才跳底"。
- 身份校验：快照记录捕获时的
  `run 集合指纹`（各 run 的 `ordinal + lastEventSeq` 摘要）。恢复时若当前 timeline
  的指纹不一致（例如该会话在后台又跑完了一个回合），判定快照过期：披露 map 仍可恢复
  （按 id 命中的子集），但滚动位置丢弃、跳底。
- 容量：Map LRU 8 个会话，超出淘汰最旧。

### 2.4 实现要点

- 快照逻辑抽成纯模块 `chat/session-view-snapshot.ts`（捕获/还原/指纹/LRU 全部可单测），
  Transcript 与 continuity 只负责在正确时机调用。
- 恢复时机：restore 数据到位后**两帧**再写 scrollTop（复用 select-then-paint 的
  two-rAF 纪律），并加"scrollHeight 稳定或 200ms 超时"保护；内容尚未撑开时写 scrollTop
  会被 clamp，必须防住。
- follow 状态：还原 `pinnedRef/lastScrollTopRef` 的等价物；`showJump` 按恢复后的
  位置重算，不恢复旧值。
- 不改变 SSE focused-only 模型；后台会话内容变化的兜底就是身份校验。

### 2.5 边界与非目标

- 不做多窗格、不做后台会话保活刷新（那是运行时文档 R5 之后的另一轮）。
- 不恢复半途的流式文本位置（离开时正在流式的会话，回来按"有新内容"处理：身份校验
  必然失败 → 跳底）。这是有意为之，避免"回来看到断在半空的流"。
- 阅读列宽是全局偏好，不进快照。

### 2.6 验证门

- vitest：`session-view-snapshot.ts` 纯单测（捕获/还原/指纹失效/LRU 淘汰/两帧恢复的
  时序模拟）。
- e2e（mock）：
  - 会话 A 滚到中部 → 切 B → 切回 A：视口位置保持，且**不出现** B 的内容（扩展既有
    stale-transcript 场景）；
  - A 在后台跑完新回合（mock 推进状态）→ 切回 A：不还原旧滚动，跳底；
  - 披露状态：A 中展开的组在切回后仍展开。

---

## 3. F2 工具与活动耗时呈现

### 3.1 背景与参考

- open-vetta：工具卡时长徽标只在"显著时长"（>1000ms）时出现；展开的 meta 行显示
  分段耗时（"download 2.1s · ocr 12.3s"）。
- PI-Desktop：处理组头部逐秒 elapsed 计时。

### 3.2 目标行为（三层，全部不依赖运行时合同）

1. **工具卡时长徽标**：`ToolCard` 读取 `ToolOutputEnvelope.protocol_metadata.duration_ms`
   （`lib/rove-types.ts:465`，字段已在线上），≥1000ms 才显示 `X.Xs` 徽标；<1000ms
   不显示（vetta 的"显著时长"原则，避免噪音）。实做补充：该字段位于 envelope，
   Web 端 `parseToolOutputEnvelope` 原本不保留 `protocol_metadata`、
   `toWorkbenchStreamEvent` 原本丢弃 `envelope`，因此本条同时补上这段投影。
2. **活动组 elapsed**：活动组激活期间头部右侧显示"已运行 Ns"，逐秒更新；
   组内最后一个完成事件到达后冻结为总时长。计时基于**事件到达时刻**（事件进入
   dispatch 包装器时用 `performance.now()` 记录），不用系统墙钟。实做补充：记录
   在 `chat/tool-timing.ts` 的模块级到达台账里，key 为 tool call id / input id ——
   dispatch 时事件只带这两个身份，entry id 需要 run scope（`run:…`/`job:…`），
   在 dispatch 处并不可得。
3. **展开详情**：工具卡展开区域在 `duration_ms` 存在时显示精确值；不存在时不显示占位。

### 3.3 关键边界：重放与到达时间失真

- 到达时间 ≠ 服务端执行时间。两类失真必须处理：
  - **SSE 重放/恢复**（`Last-Event-ID` 续传、restore 后的快照重放）：事件成批到达，
    elapsed 完全不可信。实现上只把 `stream_event` 记为到达：`reset`/`hydrate`
    清空台账，`job_state_synced`（attach 时快照重放、审批后的权威同步）不记录。
    任何一项没有实时到达的活动就没有起点，**不显示** elapsed，只认 `duration_ms`。
  - **标签页后台节流**：后台标签的 timer/事件到达都会膨胀。冻结值
    （最后完成到达 − 首个开始到达）本身可能虚高；文案用"约"，且徽标类显示优先用
    `duration_ms`。
- 计时逻辑抽纯模块 `chat/tool-timing.ts`：输入到达台账与 key 列表，输出
  `{liveElapsed | frozenMs | null}`；缺一项到达、或跨度倒退（负数）都返回 `null`，
  宁可不显示也不给一个 0 秒的假事实。禁止把这段逻辑散在组件里。

### 3.4 验证门

- vitest：`tool-timing.ts`（实时序列、重放/恢复不显示、乱序与倒退、无开始事件等）；
  徽标阈值（999/1000/2450/60000）与解析层（`protocol_metadata` 保留、非法值报错）。
- e2e（mock）：流式期间组头计时逐秒更新、重放/恢复路径不显示 elapsed、
  `duration_ms` 徽标与展开精确值显示。实做限制：mock 的 SSE 路由一次性 fulfill
  整个事件列表，无法"先开始、过一会儿再完成"，因此**冻结转换**由 vitest 序列覆盖，
  e2e 覆盖计时、重放与徽标。

---

## 4. F3 流式 markdown 的分块 memo 化

### 4.1 背景与参考（2026-09-26 核对修正）

PI-Desktop 的增量 markdown 按**顶层块**切分，只有尾块重新解析，因此 token 流入时
渲染成本恒定。核对 `chat/streaming-blocks.ts` 后确认：rove 的 `segmentMarkdown`
**不是**顶层块切分，`flushProse()` 只被**围栏开头**触发，所以只有**闭合围栏**
才是段边界：

- 纯 prose 流式（无围栏）→ 全程只有一个段，其 text 随每个 delta 变化；
- 围栏打开但未闭合 → 围栏前的 prose 段已经固定，未闭合部分与尾部 prose 合并进
  最后一个段；
- 围栏闭合 → 该块从 prose 段变成 code 段，一次重写。

因此本条能拿到的收益是"**已闭合围栏之前的段**在追加 delta 时字符串不变 → memo 命中"，
而不是"所有已完成块的渲染成本恒定"。真正做顶层块切分需要一个匹配 CommonMark 的词法器
（`marked` lexer、`streaming-markdown` 等），被 §17 拒绝；手写"按空行切 prose"会破坏
跨块结构（松散列表被拆成多个独立列表、链接引用定义 `[x]: url` 不再对前面的块生效、
引用块延迟续行丢失），风险大于收益，故**不做**，并在 4.2 如实标明限制。

### 4.2 目标行为（修正后）

- `RichTextMarkdown` 包 `React.memo`，比较该段的 text（段渲染输入只有它；kind 决定
  怎么切分，不决定怎么渲染）。
- `RichText` 用 `useMemo` 缓存 `segmentMarkdown(bounded)`；已完成段的 text 在追加
  delta 时不变 → memo 命中跳过；只有尾段（正在增长的段）重新走 ReactMarkdown。
- **已知限制**：纯 prose 长回复仍是单段重解析，本条不改变其成本（见 4.1）。
- 围栏闭合瞬间（prose 段裂成 code 段）会导致该块一次重解析——可接受，记录为已知行为。
- 不引入新依赖（Shiki/词法器等仍被 W 文档 §17 拒绝，本条不改）。

### 4.3 实现约束

- `React.memo` 的 props 必须全部稳定：不传每次渲染新建的回调；若必须传，用
  `useCallback`。禁止用 index 之外的会变化的 prop 打破 memo。
- `remarkPlugins` 与 `components` 必须是模块常量：react-markdown 在它们的 identity
  变化时也会重新解析，逐渲染新建会抵消 memo。
- `MAX_MARKDOWN_CHARACTERS = 300_000` 上限与懒加载的 `RichCodeBlock`/`MermaidDiagram`
  行为不变。

### 4.4 验证门

- vitest：渲染计数断言——用例必须包含**已闭合围栏**（否则没有"已完成块"可断言），
  挂载后追加 N 个 delta，已完成块触发的解析次数不随 N 增长（`vi.mock("react-markdown")`
  统计解析次数）。这是本仓库第一处 DOM 环境测试：`jsdom` 作为 devDependency 引入，
  现有 SSR 测试栈（`renderToStaticMarkup`）无法观测 memo 命中。
- 手测证据：长回复（数千行 markdown，含代码块）流式期间主线程无长任务
  （Performance 面板截图进实施记录）；未运行时在实施记录里如实标注为未做。

---

## 5. F4 会话 hover 卡片

### 5.1 背景与参考

PI-Desktop 的会话 hover 卡（320px portal：模型、工作区/分支、实时状态、协作预览）。

### 5.2 目标行为

- 侧栏会话行 hover 300ms 后显示 portal 卡片；移开即消失；Escape 关闭；
  不阻塞点击与键盘操作。
- 内容（只用现有 DTO 能给出的数据，`product/product-api-types.ts:117-128`）：
  标题、状态、最近更新时间、创建时间、fork 来源（父会话 + fork 点 run/seq，存在时）、
  工作区根路径。
- 模型一行：hover 时懒加载 `GET /product/sessions/{id}/model-config`（幂等、latest-wins、
  失败显示"—"，不重试风暴）。
- 仅在 `(hover: hover) and (pointer: fine)` 启用（与窄屏 hover overlay 同一纪律）；
  触屏/键盘无此卡。`role="tooltip"` + `aria-describedby` 指向行。

### 5.3 边界与非目标

- 不放"协作预览/最近消息摘要"（那需要 R5/R7 的数据面）。
- 卡内不放操作按钮（行内 hover 操作已有，避免双入口）。

### 5.4 验证门

- vitest：显隐状态机（延迟显示、提前离开取消、粘性、Escape）。
- e2e（mock）：hover 出卡片、内容正确、离开消失、键盘导航不触发。

### 5.5 实现记录（2026-09-26）

- 状态机与落位算法抽到 `sidebar/session-hover-card.ts`（`reduceSessionHoverCard`
  与 `placeSessionHoverCard`），组件只做取材与渲染。
- 模型行通过 `sidebar/session-model-summary.ts` 的进程内 store 懒加载：每会话一次
  请求、结果（含失败）缓存、失败显示"—"不重试，缓存上限 64 条。
- portal 目标是行所在的 `.product-app-frame`，而不是 `document.body`：v2 令牌与
  scoped 规则都在该元素上，落到 body 会渲染成一张没有样式的卡。仍然 portal，
  是为了躲开侧栏滚动容器的裁剪。
- "粘性"按"Escape 关闭后，只要指针没离开该行就不再打开"实现；指针离开即关闭
  （设计 5.2 的"移开即消失"）。
- 卡片 `pointer-events: none`：不拦截行点击、菜单与拖拽；键盘不触发（无 focus 分支）。

---

## 6. F5 Toast 通知层与后台失败可见性

### 6.1 背景与参考

PI-Desktop：顶部 toast（4s/8s）+ 只收失败的持久通知收件箱 + 失焦才走系统通知。
rove 现状（§1.9）：无 toast；后台失败只有侧栏红点 + 2.5s 轮询。

### 6.2 目标行为（首批 toast；收件箱不在本轮）

- 新增 `shell/toast/`：`ToastProvider`（context + reducer）。同屏最多 3 条，超出丢最旧；
  `success/info` 停留 4s、`error` 停留 8s；hover 暂停计时；可手动关闭；
  `prefers-reduced-motion` 下直接显隐无动画；`role` 按种类映射 `status`/`alert`；
  位置固定于顶部居中，不遮挡 composer。
- 接入点（首批，全部是"后台发生/跨会话发生"的事；行内提示保持不变，不重复弹）：
  - 后台会话失败：`refreshSessionStatuses` 轮询发现某会话 status 从非 error 变为
    error 时弹 toast；去重规则：同一会话 5 分钟内最多一条。
  - M1 迁移结果、fork 创建完成、provider 连接测试结果、恢复错误（partial restore）。
- 文案全部走 copy 字典（zh-CN + en-US 镜像）。
- 收件箱（铃铛 + 失败列表）：**依赖运行时文档 R5 的目录级 SSE** 才有实时性，
  列为 R5 落地后的后续项，本文档只留接口：ToastProvider 的 store 需要把 error 类
  toast 同时追加进一个可查询的内存列表，未来直接复用。

### 6.3 验证门

- vitest：队列上限、去重窗口、hover 暂停、reduced-motion 分支。
- e2e（mock）：后台会话置 error → toast 出现且 5 分钟内不重复；success 4s 自动消失。

### 6.5 实现记录（2026-09-26）

- `shell/toast/` 落地：`toast-store.ts`（上限 3 条丢最旧、success/info 4s、error 8s、
  去重窗口 5 分钟、hover 暂停、error 失败历史）+ `notifications.ts`（哪些事值得弹）+
  `ToastProvider.tsx`（定时器、`role` 映射、reduced-motion、portal 视图）+ 
  `use-background-failure-toasts.ts`。
- **接入点取舍**：§6.2 要求"行内提示保持不变，不重复弹"。因此只对**没有行内出口**的事弹
  toast：后台会话失败（侧栏只有一个红点）、fork 创建完成、以及 provider 连接测试**面板
  已经不在屏幕上**时的结果。M1 迁移结果与 partial restore 保持原有行内视图——前者有可
  关闭的完成摘要，后者有带原因列表和重试按钮的 `restore-notice`——同一屏再弹一条是重复，
  不是可见性；这条差异在此记录，而不是删掉行内提示。
- 失败才算"新闻"：本窗口第一次看到某会话就是 error 时**不弹**（可能在窗口打开前就失败
  了）；当前正在看的会话也不弹（会话自己会在行内报错）。
- 收件箱仍按 §6.2 等运行时 R5 的目录级 SSE；本轮把错题历史（`failures`，上限 50）留成
  可查询的接缝，已由单测固定。
- 去重窗口的 e2e 做法：让一个"keeper"会话长期处于 running 以维持轮询，同一会话在一个窗口
  内失败两次 + 同一轮另一个会话首次失败，断言只出现后者一条——否则断言"只有一条"会被
  "根本没弹"蒙混过关。

---

## 7. F6 smart-stop 发送快照（含粘贴 chips）

### 7.1 背景与参考

PI-Desktop 的 smart stop：abort-before-reply 时从**结构化快照**原子恢复草稿
（含文件引用 chips），绝不从 `@path` 文本反解析。rove 现状（§1.8）：恢复只有文本，
chips 在发送瞬间折叠丢失。

### 7.2 目标行为

- `composer-draft-store` 增加每会话 `lastSendSnapshot: { text, chips, at }`：
  - 发送成功时写入（text +当时的 chips 数组）；
  - 会话进入终态回合（复用 `reconcileTerminal` 的时机）且未被取消时清除；
  - 与现有 draft 同 Map、同 LRU 纪律（memory-only，不落盘——延续 W 文档
    "草稿不持久化"的决策）。
- 智能停止恢复时：快照存在 → 恢复 text **和** chips；快照缺失 → 退化为现状
  （折叠围栏文本回填）。两条路径都在 UI 上如实提示恢复来源。
- 部分中止（模型已产出部分内容）的文案分支：依赖运行时文档 R2b 的 `aborted` 标记，
  本轮前端预留判定分支但不启用（没有该标记时行为不变）。

### 7.3 验证门

- vitest：draft-store 快照写入/终态清除/降级路径。
- e2e（mock）：带粘贴 chip 发送 → 立即取消 → composer 恢复出文本 + chip。

### 7.4 实现记录（2026-09-26）

- 快照放在既有 draft 记录里（同一 Map、同一生命周期，仍然只在内存里）：`lastSend`
  由 `submit` 在**发送被接受**时写入，内容是**键入的原文 + 当时的 chips**，不是折叠后的
  消息文本——折叠不可逆，这正是要留快照的原因。
- `restoreLastSend()` 一次消费：成功恢复文本与 chips 后清空快照并记录恢复来源
  （`lastRestore.source = "snapshot"`）；没有快照时返回 `"none"`，调用方退回原来的
  折叠文本路径（`restore()`，来源记为 `"text"`，并清掉 composer 里已经和文本对不上的
  chips）。两条路径各有自己的提示文案，UI 不会假装是另一条。
- 清除时机复用 `reconcileTerminal`：`useSessionContinuity` 新增 `onTurnTerminal`
  回调，在它决定这一轮已终结、要刷新会话的同一处触发。停止路径先消费快照，所以
  "被取消的回合"不会因此丢东西；这里的 `clearLastSend` 只会丢掉真正过期的快照。
- 部分中止（模型已产出部分内容）的文案分支按 §7.2 预留：`PARTIAL_ABORT_MARKER`
  常量为 `undefined`（运行时 R2b 尚未发出该标记），`isPartialAbort()` 只在标记为真
  **且**该轮已产出内容时才为真，因此当前行为与改动前一致；单测把这两个条件都钉住，
  标记落地时只需改这一个常量。
- 终态清除无法用浏览器用例区分（发送本身就会覆盖快照），因此由 store 单测覆盖；
  e2e 覆盖的是设计门要求的 chip 恢复，以及"没有快照时退回折叠文本"这条降级路径
  （刷新页面即丢掉内存草稿，正好构造出该状态）。

---

## 8. F7 杂项清理

1. `/dev/workbench` 横幅（`app/dev/workbench/page.tsx:25-27`）声称"Benchmark lives
   under Settings → Advanced"，但 `advanced` 已是渲染 General 的路由别名且基准运行器
   已从产品 Settings 移除。修正文案或删除该横幅（决定：删除指引，横幅只保留
   dev-route 警示）。
2. `app/dev/product-ui-v2/`：保持 notFound 门控；页面顶部加显眼的
   "design mock, not product" 徽标（它已经在 dev 门后，但防止未来门控改动后被误当产品）。
3. 消息级 fork 按钮继续如实标注"复制会话"（fork 锚定最后终态 run 的
   `last_event_seq`，由服务端派生，`apps/api/src/lib.rs:2239-2340`）；消息级编辑重发
   等运行时文档 R6。
4. `use-sidebar-width.ts:140-148` 的注释记录的测量结论保留原样（是有效文档）。

验证：`pnpm typecheck && pnpm test && pnpm build`；`/dev` 相关 e2e 更新。

### 8.1 实现记录（2026-09-26）

1. `/dev/workbench` 横幅删掉"Benchmark lives under Settings → Advanced"（基准运行器
   已不在产品 Settings 里，`advanced` 只是渲染 General 的别名），只留 dev 路由警示
   "Development route only."。
2. `app/dev/product-ui-v2/` 仍是 notFound 门控，并在最顶部加了一条醒目的
   "Design mock, not product."（`role="note"`）徽标。它占了 `.preview` 网格的第一行，
   因此 `product-ui-v2.module.css` 的行模板改为 `auto 52px minmax(0, 1fr)`——移动端
   媒体查询里还有一份两行的旧模板，第一次跑 e2e 就被抓出来（product bar 被撑成
   1fr 并盖住了 "Open run evidence"），两处一起改。`product-ui-v2.spec.ts` 里原本
   `getByRole("note")` 唯一的断言改为按文案定位，并新增徽标断言。
3. 消息级 fork 按钮保持如实的"复制会话"（`chat/Transcript.tsx`，fork 锚定最后一个
   终态 run 的 `last_event_seq`，由服务端派生），消息级编辑重发仍等运行时文档 R6。
4. `shell/use-sidebar-width.ts` 里那段拖动测量的注释保留原样（是有效文档）。
5. 超出本节编号的一处清理（§0 F7 的"杂项"）：会话头的 fork 按钮原本硬编码英文
   `Fork`，与消息级按钮是同一个服务端操作，现改用同一个既有文案键
   `chat.forkSession`；`toast.spec.ts` 与 `real-api.spec.ts` 的定位符随之改为在
   `.chat-pane__header` 内按该名字查找。

---

## 9. F8 样式 token linter 与 CI 接入

### 9.1 目标

PI-Desktop 用 CI 脚本拒绝非 token 动效值进仓库（`check-style-tokens.mjs`）。rove 的
token 体系目前全靠 review 自觉。新增 `scripts/check-web-style-tokens.mjs`：

- 扫描 `apps/web/styles/**/*.css`（v1/v2/v3 三层全扫）；
- 规则（首批刻意最小）：
  1. `transition`/`animation` 的 duration 必须是 `var(--motion-duration-*)`；
  2. `transition-timing-function`/`animation-timing-function` 必须是
     `var(--motion-ease-*)` 或字面量 `linear`/`steps(...)`；
  3. `prefers-reduced-motion` 块内不受限（那里本来就要写即时值）。
- 豁免机制：行尾注释 `/* style-token: allow <理由> */`，豁免清单在 PR 描述里可见。
- 接入：`apps/web/package.json` 增 `lint:style-tokens`；文档在
  `apps/web/README`（或 package script 注释）说明；不接入 GitHub Actions 的强制门
  （仓库 CI 拓扑是另一回事），先作为本地 + PR 描述要求的门。

### 9.2 非目标

- 不禁布局尺寸的裸 px（那是设计自由度，不是动效一致性）。
- 不做自动修复。

### 9.3 验证

- 对现存三份 CSS 跑一遍：产出存量违规清单，**逐条豁免或修正**后脚本必须全绿
  （不允许"脚本带着存量红跑"）。

### 9.4 实现记录（2026-09-26）

`scripts/check-web-style-tokens.mjs`（含 `node --test` 自测
`scripts/check-web-style-tokens.test.mjs`，14 个用例）；`apps/web/package.json` 增
`lint:style-tokens`（先跑自测再扫）；`apps/web/README.md` 的 Verification 一节说明
规则、豁免与"只是本地门、不接入 CI"。

首次跑出的存量违规 29 条，逐条处理如下。

修正（v2，product-v2.css；动效值不变，只是改为引用 token）：

| 位置 | 原值 | 现用 |
|---|---|---|
| button 基础 transition（5 个属性） | `140ms ease` | `--motion-duration-fast` + `--motion-ease-standard` |
| status-dot / streaming 点脉冲 ×2 | `1.8s ease-in-out` | `--motion-duration-pulse` + `--motion-ease-in-out` |
| toast 进场 | `160ms ease-out` | `--motion-duration-fast` + `--motion-ease-out` |
| 移动抽屉 sidebar / inspector ×2 | `180ms cubic-bezier(0.16, 1, 0.3, 1)` | `--motion-duration-collapse` + `--motion-ease-emphasized` |
| session spinner | `1s linear` | `--motion-duration-spin` + `linear` |
| 侧栏滑入 / 滑出 ×2 | `240ms` | `--motion-duration-slide` |
| composer meta 折叠 | `180ms` | `--motion-duration-collapse` |
| activity 折叠箭头 | `160ms ease` | `--motion-duration-fast` + `--motion-ease-standard` |
| activity 阶段点脉冲 | `1.6s ease-in-out` | `--motion-duration-activity-pulse` + `--motion-ease-in-out` |
| Settings 区块边框呼吸 | `900ms ease-in-out 4` | `--motion-duration-breathe` + `--motion-ease-in-out` |

新增 6 个 duration token（spin/pulse/activity-pulse/breathe/slide/collapse）与 2 个
ease token（in-out/emphasized），都放在既有 motion token 块里：循环周期也是动效
语义，应该由 token 层拥有。除 button 的 `140ms ease` → `150ms` 与
`--motion-ease-standard`（设计 token 里最近的既有值，肉眼无差别）外，其余数值与
曲线不变。

豁免（v1，product.css；v1 是冻结皮肤，F1–F5 都刻意不给它加新样式，因此不给 v1
引入 motion token 层，改为逐条带理由豁免）：

| 位置 | 值 | 理由 |
|---|---|---|
| `.inspector-skeleton` shimmer | `1.35s linear` | v1 皮肤自带的骨架屏周期，v1 冻结 |
| `.tab-button` / 侧栏项 / 记忆面板项 ×3 | `160ms ease` | v1 皮肤自带的交互时长与曲线，v1 冻结 |

`prefers-reduced-motion` 块内的 `0.01ms !important`（v1/v2/v3 各一处）按规则 3 自动
豁免，不是例外清单的一部分。脚本输出的豁免清单是 4 条，即上表。

规则细节（与 9.1 的三条一致，实现时明确下来的部分）：简写里每个逗号段的第一个
时间值是 duration、第二个是 delay（delay 允许字面量，且 `0s`/`0` 允许）；comma 与
空白切分都尊重括号，因此 `cubic-bezier(0.2, 0, 0, 1)`、`steps(2, end)`、
`var(--x, 1ms)` 都是单个 token；`@keyframes` 体内的
`animation-timing-function` 是逐帧曲线、不是组件动效，不扫；豁免注释放声明自身
任一行或紧邻上一行都算，无理由的豁免与"豁免了不存在违规"的豁免本身报错，避免
豁免清单腐化。

---

## 10. F9 侧栏折叠动效合成器友好化

- 现状（§1.11）：grid 轨道宽度动画导致中列全程 reflow。
- 步骤：
  1. 先测量：复用 `docs/plans/2026-09-22-ui-shell-layout-remediation.md` §8.1C 的
     测量方法，记录折叠动画期间的 script/layout 耗时基线（进实施记录）。
  2. 若 reflow 成本显著：去掉 `grid-template-columns` 的 transition（轨道一步到位），
     保留侧栏 `transform + opacity` 的 240ms 滑出；补齐 `:3782-3783` 注释中标记的
     `inert`/`aria-hidden` 门控。视觉从"连续推挤"变为"侧栏滑出、内容即切"——
     作为记录在案的偏差接受。
  3. 测量显示成本可忽略 → 不改，把数据写进实施记录即可（"不在未复现时就改"的纪律）。

### 10.4 实现记录（2026-09-26）

**测量**。复刻 §8.1C 的方法（CDP `Performance.getMetrics`，同一台机器、`next dev`、
1280×720、30-run transcript 的长会话），对**同一次折叠/展开动作**取 6 次平均，扣掉等长空转窗口
（空转窗口 Script/Layout/Recalc 均为 0，故净值≈原值；一次交互占用 700ms 窗口）：

| 每次折叠/展开 | ScriptDuration | LayoutDuration | LayoutCount | RecalcStyleDuration | RecalcStyleCount | TaskDuration |
|---|---|---|---|---|---|---|
| 轨道动画（改前） | 55.0 ms | 18.9 ms | 15.2 | 33.4 ms | 24.5 | 160.4 ms |
| 轨道一步到位（改后） | 44.9 ms | **1.8 ms** | **2** | **6.0 ms** | 13.3 | 75.8 ms |

即：布局次数 15.2 → 2（−87%），布局耗时 18.9 → 1.8 ms（−90%），样式重算 33.4 → 6.0 ms。
长任务计数两种形态都是 0（单帧未超 50ms），但逐帧 reflow 的成本真实存在且与设计 §1.11 的
判断一致，因此按步骤 2 落地。

**改动**（`apps/web/styles/product-v2.css`）：

- 删除 `.product-body:not([data-settings="true"])` 上的
  `transition: grid-template-columns ...`（两处对应注释写明测量值与原因）；
- 侧栏基础规则补 `position: relative; z-index: 2; width: var(--sidebar-nav-width)`：轨道在
  一帧内变成 0 后，拉伸的 grid item 会在同一帧被压成 0 宽而"没有东西可滑"，固定宽度让它保持
  240px 并从对话区上方滑出（折叠态已是 `pointer-events: none`，滑出过程不抢点击）；
  这三条写在**基础规则**里而不是新规则里，是因为 `max-width: 960px` 抽屉块与它同特异性且在后，
  这样 ≤960px 仍然由抽屉接管（已在 820px 视口实测：`position: fixed`、320px、`z-index: 42`）。

**偏差（记录在案）**：视觉从"连续推挤"变为"轨道一步到位、侧栏滑出、内容即切"。折叠瞬间对话列
即占满整行，侧栏在这 240ms 内从其上方滑走（240ms `transform + opacity`，`--motion-duration-slide`）。
`inert`/`aria-hidden` 门控**无需补**——设计原文要求补齐的那一项已在
`apps/web/sidebar/WorkspaceTree.tsx:268-270` 存在（`data-collapsed` + `aria-hidden` +
`inert`），实施时逐条核对并已写进 e2e 断言。

**回归**：`apps/web/tests/e2e/layout.spec.ts` 新增
"the rail's collapse snaps the track and slides the rail itself"：断言
`.product-body` 的 `transition-property` 不含 `grid-template-columns`、侧栏自身保留
`transform` 过渡、折叠后对话列 `x≈0`、侧栏仍是 240px 且 `z-index: 2`、折叠态
`aria-hidden="true"` + `inert`、重开后对话列从 240px 起、侧栏最终回到 `x≈0`。同一文件里
原有的"折叠侧栏不占列"断言改为对 `main.x` 断言，并对侧栏自身位置用有界轮询等到滑出结束
（滑出是真实动画，瞬时采样不再是稳定读数——这不是放宽断言，而是把"不占列"与"已滑走"分开测）。

**未做**：手动 Performance 面板长任务截图仍未做（与 F3 同一遗留项）；本项的长任务计数为 0，
因此这里没有"长任务"证据可提供。

---

## 11. F10 偏好小项

1. **per-model reasoning 记忆**（open-vetta 的 `reasoningByModelAtom`）：
   `QuickModelControl` 选择 reasoning 时按模型 id 记忆（localStorage
   `rove.ui-reasoning-by-model`），换模型时带出上次选择；CAS 冲突恢复沿用现有逻辑。
   仅 UI 记忆，不改会话 model-config 合同。
2. **会话自动标题（前端版）**：新会话首次发送成功后，若标题仍为空/默认模板，
   取首条用户消息前 48 码点（截断加省略号）调用现有 update-title 端点写一次；
   之后任何手动重命名都优先生效（本地不再覆盖）。判定"从未手动命名"只能靠
   "标题仍等于默认模板串"近似——记录该偏差；服务端摘要标题在运行时文档 R10 登记。

验证：两项各自 vitest + 一条 e2e。

### 11.3 实现记录（2026-09-26）

**F10.1 per-model reasoning 记忆**。新增 `apps/web/product-v2/reasoning-memory.ts`：
`rove.ui-reasoning-by-model` 是一个 model id → reasoning 的 localStorage 映射，纯函数
（`parseReasoningByModel` / `rememberReasoningInMap` / `reasoningForModel`）与存储 I/O
（`readReasoningByModel` / `rememberReasoning`）分离，后者吞掉被禁用或写满的 storage——
这是便利偏好，不该让一次保存失败。映射有界（`REASONING_BY_MODEL_LIMIT = 50`，最旧的先走），
并且逐条校验：非字符串、未知 reasoning 值、空 model id、非对象 JSON 全部丢弃，手改过的
localStorage 不可能把非法值送进请求。

接线在 `QuickModelControl`：选 reasoning 时按**当前输入框里的 model id** 记忆；换模型
（输入框直接改，或切 profile 带出其 default model）时先查记忆，命中就带回该模型的
上次选择，未命中才走原有回退规则（仍在本就支持 reasoning 的 provider 上就保留当前值，
切到不支持的服务则回退 `default`）。服务端 model-config 合同未改，CAS 冲突恢复路径
（`onModelConfigChange` 失败后的既有处理）未改。datalist 里输入但 provider 未上报的
model 仍然禁用 reasoning 选择——这一条是既有能力判定（`quickModelReasoning`），本项不动。

证据：`apps/web/product-v2/reasoning-memory.test.ts`（7 例：隔离、覆盖、有界、脏数据、
storage 不可用）；`apps/web/tests/e2e/session-preferences.spec.ts`
"the reasoning effort follows the model it was chosen for"（两个 openai-responses profile，
各自记住 high/low，来回切换两次都能带回）。

**F10.2 会话自动标题——设计前提修正**。设计原文按"尚未实现"描述，实际上
`state/use-session-continuity.ts` 已经有一版：标题严格等于 `"New session"` 时**发送前**写入，
截断用 `slice(0, 42)`（UTF-16 码元）。因此本项落地的是**参数修正**，并逐条记录：

| 项 | 原有 | 现在 |
|---|---|---|
| 截断 | 42 个 UTF-16 码元（`slice`，可能劈开代理对） | 48 个码点（`Array.from`），未超长时不加省略号 |
| 时机 | 发送请求**之前**（被拒绝的消息也会命名会话） | 服务端接受该消息**之后** |
| 未命名判定 | 仅等于字面量 `"New session"` | 空标题也算未命名；`DEFAULT_SESSION_TITLE` 单点定义并注明与 `apps/api/.../validation.rs` 同源 |
| 手动命名优先 | 依赖发送前的瞬时快照 | 写入前**重新读一次 catalog**：请求在途期间发生的重命名优先；另外本页对该会话只自动命名一次（避免"服务端标题回执到达前的第二条消息"改写首条消息的标题） |

新增 `apps/web/state/session-auto-title.ts`（`DEFAULT_SESSION_TITLE` /
`hasDefaultSessionTitle` / `autoSessionTitle` / `AUTO_TITLE_MAX_CODE_POINTS = 48`），
`use-session-continuity.ts` 的 `send` 在 `accepted` 之后调用一次；原 `truncateTitle` 删除。

证据：`apps/web/state/session-auto-title.test.ts`（7 例：占位标题/空标题、空白折叠、
48 码点截断、恰好 48 不加省略号、emoji 不被劈开、空消息返回 null）；
`apps/web/tests/e2e/session-preferences.spec.ts`
"the first message names a session the user never named"（新会话首条消息 → 服务端标题与
头部 `<h1>` 都变成截断值；已有标题的会话发送后标题不变）。

**仍然记录的偏差**："从未手动命名"只能近似为"标题仍等于默认模板串"（有人把会话改名回
`New session` 就会被再次自动命名）；服务端摘要标题仍是运行时文档 R10 的登记项，本项不做。

---

## 12. F11 服务端会话搜索 `q` 接线（P2）

- 现状（§1.10）：目录已把会话拉全，本地子串 ≈ 服务端 `q`（title LIKE）。
- 本项的真实意义是**解除"目录全量加载"假设**：目录超过页上限时本地过滤失真。
- 设计：输入非空且 180ms 防抖后，对每个工作区并发调
  `listSessions({ q, limit: 200 })`（latest-wins 序列失效沿用现有模式），结果替换
  该工作区列表；本地子串保留为无网降级。差异记录：服务端 LIKE 只做 ASCII 大小写
  折叠，英文与本地 `toLocaleLowerCase` 有别；服务端结果为权威。
- P2 的原因：在 R5（目录 SSE）与真实大目录出现之前收益有限；不做也不会错。

### 12.1 实现记录（2026-09-26）

**前提复核**：`state/server-product-state.ts:66-88` 的 `listWorkspaceSessions` 本来就会
逐页翻到游标结束（上限 `MAX_SESSION_PAGES_PER_WORKSPACE = 64` 页），所以"目录已把会话
拉全"在当前代码里成立——本地子串与服务端 `q` 在正常情况下同解。本项因此不是修 bug，而是
按设计解除"目录全量加载"假设，并保留本地过滤作为无网降级。

落地：

- `sidebar/session-server-search.ts`：纯逻辑。`SESSION_SERVER_SEARCH_LIMIT = 200`；
  `serverSearchQuery` 去空白并把空查询判为"无需询问"；`resolveSessionRows` 是唯一裁决点
  ——空查询恢复全部已加载行；工作区名命中时该工作区不过滤（保持原行为）；**服务端答了就
  以服务端行为准**（包括"服务端答 0 条"也算无匹配，不再回退本地）；没有服务端答案
  （未询问/在途/失败）才用既有的 `filterSessions` 本地子串。
- `sidebar/use-server-session-search.ts`：输入非空后沿用既有 `SESSION_SEARCH_DEBOUNCE_MS`
  （180ms），对每个工作区并发 `listSessions({ q, limit: 200, includeArchived: false })`；
  序列失效复用 `session-search.ts` 已有的 `nextSearchToken`/`isCurrentSearch`（latest-wins），
  慢响应不会覆盖新查询；任一请求失败即清空服务端结果并置 `unavailable`。工作区 id 用
  连接串做依赖键，避免父组件每次渲染的新数组触发重查。
- 接线：`use-server-product-state.ts` 暴露 `searchSessions`（`fromProductSession` 映射），
  `ProductApp` 传给 `WorkspaceTree` 的可选 `searchSessions` prop；未传时行为与之前完全一致。
  侧栏搜索说明文案改为反映真实范围（`workspace.searchScope`），失败时改用
  `workspace.searchUnavailable` 明说"服务端检索不可用，已改为只过滤已加载的名称"。

**差异记录（按设计）**：服务端 `LIKE` 只折叠 ASCII 大小写，本地用 `toLocaleLowerCase`；
非 ASCII 标题两者可能不同，此时**服务端结果为权威**（`resolveSessionRows` 的取舍）。
另记录：本次只取一页（`limit: 200`），不翻页；服务端返回的会话不写入目录缓存，只用于本次
渲染，因此目录的权威性不变。

证据：`apps/web/sidebar/session-server-search.test.ts`（6 例，覆盖空查询、工作区名命中、
无答案降级、服务端行为准、服务端空答案不回退）；`apps/web/tests/e2e/session-server-search.spec.ts`
两例——"只有服务端知道的会话出现在搜索里"（mock 的 `searchOnlySessions` 模拟超出目录页
上限的会话，并断言请求带 `q=checklist&limit=200`）、"服务端检索失败时本地过滤接手"。

---

## 13. F12 决策点（默认不做）

| 项 | 参考来源 | 默认理由 | 若要做的前提 |
|---|---|---|---|
| 发送键箭头↔停止 morph + 流式涟漪 | open-vetta `send-button.css` | 纯 delight；rove 已有 180ms pending capsule | 用户拍板；reduced-motion 必须关闭涟漪 |
| phrase-level 流式淡入 | open-vetta `streaming-reveal.ts` | W 文档以"chunk 驱动无积压"否决；vetta 的自适应节奏（间隔=400ms/(积压+1)，无积压时即时）解决了该理由，但仍是观感项 | 用户拍板；实现必须带积压自适应，否则禁做 |
| 跨引擎 e2e + 视觉回归基线 | rove 自身缺口（shell remediation §8.6E） | 基建成本高，与对齐无关 | 独立立项；先加 webkit/firefox 冒烟 project |
| 失焦系统通知（Tauri） | PI | 依赖桌面宿主能力面 | desktop-transport 扩展 + 独立评审 |

---

## 14. F13 附件/图片上传 UI（占位）

被运行时文档 R8 阻塞：没有上传/引用合同之前，任何附件 UI 都是假入口（违反本仓库
"布局设计不能授予能力"的纪律）。R8 合同冻结后另立前端实施计划（上传控件、
进度/失败态、vision 可用性提示、与粘贴 chip 的关系）。本节只登记，不设计。

---

## 15. 明确不做（尊重既有记录决策，实施者不得顺手做）

以下条目在既有文档中已记录为"不移植/不做"，本文档不改判：

- 装饰套件（Aurora/Orb/宠物/成就）、theme-sdk/module-federation 主题机制、jotai、
  tailwind/iconify 迁移（09-17 设计 §9）；
- PI 的 120s 审批倒计时与到期自动拒绝（rove 无该服务端合同）；
- `、`→`/` IME 重写（桌面特例，W 文档 §12.2）；
- 草稿落盘持久化（敏感文本，W 文档 §17）；
- 全局 `:focus-visible { outline: none }`（rove 要求可见焦点）；
- 思考块折叠、子代理嵌套卡片、重试倒计时（**除非**运行时文档对应条目先落地：
  thinking 通道、parentToolCallId、R2c 的 ProviderRetry 事件）；
- 货币成本显示（PI 自己也不显示）；
- 会话拖拽重排/多选/批量、密度设置、代码块换行开关、markdown/纯文本复制区分
  （W 文档 §17）；
- `animation-duration: 0.01ms` reduced-motion 惯用法（无 `animationend` 监听，
  shell remediation §8.2）。

---

## 16. 里程碑与 PR 拆分建议

每个 PR 遵循 `CONTRIBUTING.md`（feature worktree、`type(scope): subject`、
验收门、PR 描述写范围上限与非目标）。

| PR | 内容 | 门 |
|---|---|---|
| PR-A | F2（tool-timing 纯模块 + UI）+ F3（memo 化） | vitest、typecheck、build、e2e（含新场景） |
| PR-B | F1（会话切换快照） | 同上 + 滚动还原 e2e |
| PR-C | F4 hover 卡 + F5 toast + F7 清理 | 同上 |
| PR-D | F6 发送快照 + F10 偏好小项 | 同上 |
| PR-E | F8 token linter（含存量清理）+ 文档 | 脚本全绿 + `git diff --check` |

P2 项（F9/F11/F12 若拍板）各自独立小 PR，不搭车。

## 17. 验收矩阵（实施时逐行填证据）

| 条目 | 单测 | e2e/浏览器证据 | 备注 |
|---|---|---|---|
| F1 | `session-view-snapshot` | 切换往返滚动保持 / 身份失效跳底 / 披露保持 | |
| F2 | `tool-timing` + 徽标阈值 | 计时更新/冻结/重放不显示 | |
| F3 | 渲染计数断言 | 长回复流式无长任务（手测截图） | |
| F4 | 显隐状态机 | hover 内容/键盘不触发 | |
| F5 | 队列/去重/暂停 | 后台失败 toast | |
| F6 | 快照生命周期 | chip 恢复 | |
| F7 | — | /dev 场景更新 | |
| F8 | —（脚本自身有自测） | 存量全绿 | |
| F9 | — | 测量数据（改或不改都记录） | |
| F10 | 两项单测 | 各一条 e2e | |

## 18. 风险登记

- F1 的两帧恢复与既有 restore 流水线的时序竞争（generation 守卫必须覆盖快照恢复路径，
  防止"旧会话快照写进新会话"——这是本条目最大的风险点，单测必须覆盖）。
- F2 的到达时间失真误用（重放批次判定错了会把假时长给用户——宁可不显示）。
- F3 的 memo 被不稳定 props 打破后无感退化（渲染计数单测是唯一防线，不能删）。
- F5 的 toast 噪音化（首批判：只做后台/跨会话事件，不做行内提示的复制弹窗）。
