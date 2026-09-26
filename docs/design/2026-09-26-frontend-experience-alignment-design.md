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
| F3 | 流式 markdown 的分块 memo 化 | P0 | 无 | Proposed |
| F4 | 会话 hover 卡片 | P1 | 无 | Proposed |
| F5 | Toast 通知层与后台失败可见性 | P1 | 无（收件箱部分依赖 R5） | Proposed |
| F6 | smart-stop 发送快照（含粘贴 chips） | P1 | 无 | Proposed |
| F7 | 杂项清理（/dev 文案、mock 页标注、降级语义如实标注） | P1 | 无 | Proposed |
| F8 | 样式 token linter 与 CI 接入 | P1 | 无 | Proposed |
| F9 | 侧栏折叠动效合成器友好化 | P2 | 无 | Proposed |
| F10 | 偏好小项（per-model reasoning 记忆、会话自动标题） | P2 | 无 | Proposed |
| F11 | 服务端会话搜索 `q` 接线 | P2 | 无 | Proposed |
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

1. **工具卡时长徽标**：`ToolCard` 读取 `result.metadata.duration_ms`
   （`lib/rove-types.ts:465`，字段已在线上），≥1000ms 才显示 `X.Xs` 徽标；<1000ms
   不显示（vetta 的"显著时长"原则，避免噪音）。
2. **活动组 elapsed**：活动组激活期间头部右侧显示"已运行 Ns"，逐秒更新；
   组内最后一个完成事件到达后冻结为总时长。计时基于**事件到达时刻**（事件进入
   reducer 时用 `performance.now()` 记录到 ref Map，key 为 entry id），不用系统墙钟。
3. **展开详情**：工具卡展开区域在 `duration_ms` 存在时显示精确值；不存在时不显示占位。

### 3.3 关键边界：重放与到达时间失真

- 到达时间 ≠ 服务端执行时间。两类失真必须处理：
  - **SSE 重放/恢复**（`Last-Event-ID` 续传、restore 后的快照重放）：事件成批到达，
    elapsed 完全不可信。实现上由 `run-controller` 标记"重放批次"（attach 时快照重放
    与实时流的分界已知），重放批次内的条目**不显示** elapsed，只认 `duration_ms`。
  - **标签页后台节流**：后台标签的 timer/事件到达都会膨胀。冻结值
    （最后完成到达 − 首个开始到达）本身可能虚高；文案用"约"，且徽标类显示优先用
    `duration_ms`。
- 计时逻辑抽纯模块 `chat/tool-timing.ts`：输入到达序列（含 replay 标记），输出
  `{liveElapsed | frozenMs | null}`。禁止把这段逻辑散在组件里。

### 3.4 验证门

- vitest：`tool-timing.ts`（实时序列、重放批次、乱序完成、无开始事件等）；
  ToolCard 徽标阈值（999/1000/1001）。
- e2e（mock）：流式期间组头计时更新、完成后冻结、重放路径不显示 elapsed、
  `duration_ms` 徽标显示。

---

## 4. F3 流式 markdown 的分块 memo 化

### 4.1 背景与参考

PI-Desktop 的增量 markdown：按顶层块切分，只有尾块重新解析，每块 memoized，
token 流入时渲染成本恒定。rove 已有等价的切分（`segmentMarkdown`），缺的是 memo。

### 4.2 目标行为

- `RichTextMarkdown` 包 `React.memo`，比较 `content` 等渲染输入。
- `RichText` 每次渲染仍重算 `segments`（字符串切分便宜），但已完成段的 content
  字符串在追加 delta 时不变 → memo 命中跳过；只有尾段（正在增长的段）重新走
  ReactMarkdown。
- 围栏闭合瞬间（prose 段裂成 code 段）会导致该块一次重解析——可接受，记录为已知行为。
- 不引入新依赖（Shiki/词法器等仍被 W 文档 §17 拒绝，本条不改）。

### 4.3 实现约束

- `React.memo` 的 props 必须全部稳定：不传每次渲染新建的回调；若必须传，用
  `useCallback`。禁止用 index 之外的会变化的 prop 打破 memo。
- `MAX_MARKDOWN_CHARACTERS = 300_000` 上限与懒加载的 `RichCodeBlock`/`MermaidDiagram`
  行为不变。

### 4.4 验证门

- vitest：渲染计数断言——挂载后追加 N 个 delta，已完成块的组件渲染次数不随 N 增长
  （在测试中用包装组件统计渲染次数）。
- 手测证据：长回复（数千行 markdown）流式期间主线程无长任务（Performance 面板截图
  进实施记录）。

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

---

## 12. F11 服务端会话搜索 `q` 接线（P2）

- 现状（§1.10）：目录已把会话拉全，本地子串 ≈ 服务端 `q`（title LIKE）。
- 本项的真实意义是**解除"目录全量加载"假设**：目录超过页上限时本地过滤失真。
- 设计：输入非空且 180ms 防抖后，对每个工作区并发调
  `listSessions({ q, limit: 200 })`（latest-wins 序列失效沿用现有模式），结果替换
  该工作区列表；本地子串保留为无网降级。差异记录：服务端 LIKE 只做 ASCII 大小写
  折叠，英文与本地 `toLocaleLowerCase` 有别；服务端结果为权威。
- P2 的原因：在 R5（目录 SSE）与真实大目录出现之前收益有限；不做也不会错。

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
