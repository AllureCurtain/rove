# 前端优雅性参考：Kun GUI 拆解与 rove Web 差距清单

## 状态

| 字段 | 值 |
|---|---|
| 类型 | 参考分析（Reference analysis），非设计提案 |
| 实现状态 | **Reference analysis / Partially implemented** —— 本文列出的机制有明确缺陷、部分实现和未验证项，不能整体视为未实现 |
| 日期 | 2026-08-09 |
| 对照对象 | `KunAgent/Kun` @ `master`（浅克隆核实，5.9k star，1816 commits） |
| 对照基线 | rove `main`，`apps/web/` 当前代码 |

本文不构成任何架构承诺。凡涉及 rove 现状的描述均标注了文件与行号，可复核。
凡涉及 Kun 的描述均来自其源码而非 README。

---

## 0. 这份文档解决什么问题

"Kun 的界面好看、流式回复优雅"是一个可拆解的工程结论，不是审美玄学。
本文把那种"优雅"还原成 **9 个具体机制**，每一个都：

- 不依赖 Electron，可直接移植到我们的 Next.js
- 标明 rove 当前对应位置的实际状态
- 标明实现量级

同时汇总 rove Web 前端**做得不好的地方**（第 11 节），作为后续前端工作的输入。

> 完整 full-delivery 之后，本文件仍只是一份表现层参考分析。它不改变
> Engine、AgentDefinition、ToolRegistry、Artifact authority 或安全边界，
> 也不替代 [`2026-08-10-post-full-delivery-productization.md`](../plans/2026-08-10-post-full-delivery-productization.md)。
> 本文只负责核对当前 Web 实现、记录问题和提供参考证据。优先级、量级、
> “建议”及第 17 节均不产生未来任务；之后唯一可执行的要求以 2026-08-10
> productization 文档正文为准。

---

## 1. 法律边界（先读这一节）

**Kun 的许可证是 PolyForm Noncommercial License 1.0.0**，仅限学习、研究、非商业使用。
其 `packages/provider-catalog/package.json` 里逐包标注了 `"license": "PolyForm-Noncommercial-1.0.0"`。

因此：

- **不得**将 Kun 的任何代码复制、粘贴、改名后放进 rove —— 包括本文引用的那些短函数
- 可以学**模式与思路**，按自己的理解从零重写
- 本文引用的代码片段仅用于说明机制，**不是可复用的实现**

本仓库根目录存在 `软件著作权申请资料`，这条边界尤其需要严格遵守。

**第三方 OSS 库不受 Kun 的许可证约束**，但 `streamdown`、`shiki`、
`@streamdown/math`、`rehype-harden` 仍需分别完成许可证、维护状态、供应链、
bundle 体积和安全核实。本文没有完成这些核实，因此不能据此直接增加依赖。

---

## 2. 核心机制一：Typewriter pacing（视觉节奏与 SSE 分块解耦）

**这是整个流式体验最核心的一招，也是性价比最高的一项。**

### 2.1 问题本质

SSE chunk 的大小由 provider 决定，不可控。一个大 chunk 到达时会横跨多个 markdown 块，
导致"整段文字砰一下同时出现"。视觉上读起来像是页面在打补丁，而不是在打字。
慢模型时又可能一个字一个字挤出来，节奏同样难看。

**关键洞察：不要让渲染节奏等于数据到达节奏。** 数据该多快就多快，渲染自己控速。

### 2.2 Kun 的解法

位置：`src/renderer/src/components/chat/StreamdownAssistant.tsx`

用 `requestAnimationFrame` 循环去"追赶"目标长度，每帧揭示积压量的一个比例：

```ts
const CATCHUP_DIVISOR = 8      // 每帧揭示积压的 1/8
const MAX_STEP_PER_FRAME = 32  // 但设硬上限

function nextVisibleLength(current: number, target: number): number {
  if (current === target) return current
  if (current > target) return target   // 文本变短（中断/重置）→ 直接吸附，绝不倒放
  const backlog = target - current
  return current + Math.min(MAX_STEP_PER_FRAME, Math.max(1, Math.ceil(backlog / CATCHUP_DIVISOR)))
}
```

**"按比例追赶"而不是"固定速率"是设计要点**：慢模型时逐字浮现；
快模型或积压（切回标签页、恢复会话、突发大 chunk）时自动加速成快速打字，
而不是变成一堵墙，也不会卡成慢动作。`MAX_STEP_PER_FRAME` 保证极端积压时仍是"快速打字"观感。

### 2.3 三个体现成熟度的细节

1. **初始值取当前长度**：`useState(() => text.length)`。中途重入会话不会把已经在屏幕上的
   内容从头重放一遍。这是恢复历史会话时的正确行为。

2. **追上后 React 自动 bail out**：`setVisibleLength` 返回同一个值时 React 跳过重渲染，
   空转期成本只剩一个 rAF 回调。不需要额外写"是否需要继续动画"的判断。

3. **按 grapheme 切分，不按 code unit**：用 `Intl.Segmenter({granularity:'grapheme'})`，
   并对没有 `Intl.Segmenter` 的环境写了 fallback，显式处理
   代理对（surrogate pair）、组合符（`\p{Mark}`）、变体选择符（`\p{Variation_Selector}`）、
   零宽连接符（ZWJ, `0x200d`）。

   **这一点对中文场景是硬需求**：按 UTF-16 code unit 截断会把 emoji 劈成两半渲染成
   替换字符，中文组合字符同样会碎。

### 2.4 rove 现状

**缺少节奏控制。** `apps/web/chat/Transcript.tsx:167` 直接把累积的 `item.message.content`
交给 `RichText`，每个 delta 触发一次 react-markdown 全量重解析，无任何节奏控制。
流式文本按 SSE 分块大小成块出现。

### 2.5 量级

约 60 行（含 grapheme 边界处理）的核心 hook，但真实工作量还包括长 Markdown
性能、完成/取消时立即 flush、后台标签页积压、reduced-motion 和浏览器测试。
如果每帧触发完整 Markdown 重解析，成本可能高于当前 SSE 节奏，必须先测量。

---

## 3. 核心机制二：流式滚动锚定（rove 此处有一个确定缺陷）

### 3.1 Kun 的解法

位置：`src/renderer/src/components/chat/use-timeline-scroll.ts`（抽成独立 hook，不混在组件里）

```ts
const STICK_TO_BOTTOM_PX = 96   // 距底 96px 内视为"贴底"，新内容才自动跟随
const TOP_LOAD_TRIGGER_PX = 120 // 距顶 120px 触发加载更早轮次
```

三件事：

1. **贴底判定带阈值**：只有用户本来就在底部附近才自动跟随。用户往上翻看历史时
   绝不把他拽回底部 —— 这是尊重用户意图的基本礼貌。

2. **内容变化本身是滚动触发条件**：依赖里带 `scrollDeps: { contentKey, streaming, userTurnKey }`。
   不只看"有没有新条目"，还看"内容有没有变长"。

3. **用 `useLayoutEffect` 处理"用户刚发消息"**：抢在 paint 之前把 `stickToBottom` 意图
   置为 true。所以即使用户正在翻旧历史，按下 Enter 后视图会跳到底部看自己刚发的消息。
   源码注释标了 `issue #603` —— 这是踩坑修出来的，不是一开始就想到的。

4. **prepend 位置补偿**：加载更早轮次前先记 `{scrollHeight, scrollTop}`，
   内容前插后按差值补偿 `scrollTop`，避免视口跳动。

### 3.2 rove 现状：确定的缺陷

`apps/web/chat/Transcript.tsx:46-60`：

```ts
const itemCount = timeline.reduce((total, group) => total + group.items.length, 0);

useEffect(() => {
  // ...
  transcript.scrollTop = transcript.scrollHeight;
}, [atLatest, itemCount]);
```

依赖只有 `[atLatest, itemCount]`。

**流式期间 assistant message 是同一个 item**，`content` 在增长而 `items.length` 不变，
所以 effect 不会重跑。

**实际症状**：流式文字往下长的时候视图不跟随；一直到出现新的 tool call 或新消息
（`itemCount` 变化）才猛地跳一下。长回复时用户会看着文字长出视口外。

`atLatest` 的判定逻辑本身是对的（`Transcript.tsx:69`，阈值 48px），
缺的是把内容长度纳入 effect 依赖。

### 3.3 量级

修缺陷约 10 行。抽成完整 hook（含 prepend 补偿）约 120 行。

---

## 4. 核心机制三：过程折叠 + 人类可读摘要（信息密度差距最大处）

**这是"看起来优雅"最大的单一来源。**

### 4.1 问题本质

一次 20 步的 agent 运行，如果每个 tool call 渲染一张卡片，用户面对的是 20 张卡片的瀑布。
即使每张卡片都很精致，整体依然是噪音。**优雅的前提是信息密度，不是控件美化。**

### 4.2 Kun 的解法

位置：`src/renderer/src/components/chat/message-timeline-process.tsx`（1835 行）

**第一步，把 blocks 归成四类 `ProcessSection`：**

```ts
export type ProcessSection = {
  id: string
  kind: 'reasoning' | 'execution' | 'output' | 'subagent'
  blocks: ChatBlock[]
}
```

**第二步，为整段生成一句自然语言摘要。** `summarizeProcessWork()` 按 `toolKind` 分桶计数，
单复数分别取 i18n key，用 `' · '` 连接：

```
读取 3 个文件 · 搜索 2 次 · 编辑 1 个文件 · 运行 2 条命令
```

分桶维度：read / search / file_change / command / background command / approval。

**第三步，默认折叠，点开才看细节。** 进行中的那一行加 shiny-text 效果（见第 8 节）。

**第四步，同轮 subagent 合并。** 同一 `parentTurnId` 的多个非 explore 委派
coalesce 成一个 swarm 区块，而不是并列 N 张卡。

### 4.3 效果对比

| | rove 当前 | Kun |
|---|---|---|
| 20 步运行的渲染量 | 20 张 `ToolCard` | 1 行摘要（可展开） |
| 默认展开策略 | running / error / 有 mutations 时展开（`Transcript.tsx:294`） | 全部折叠，仅进行中高亮 |
| 面向对象 | 开发者视角事件流 | 产品用户视角工作叙述 |

### 4.4 rove 现状

`apps/web/chat/Transcript.tsx:288-330` 的 `ToolCard`：每个 tool call 一张卡片平铺，
卡片头显示 `tool.name` 与 `running/ok/error` 状态。

`Transcript.tsx:210-213` 左侧 meta 列还渲染了 `item.entry.eventSeq`
—— 这是**开发者调试信息出现在产品主界面**。它属于 Inspector，不属于 transcript。

### 4.5 我们的有利条件

改造成本比看起来低。`ToolCallView` 已经有分桶所需的信息：

- `tool.metadata.read_only`（`Transcript.tsx:309`）
- `tool.metadata.workspace_changed`（同上）
- `tool.mutations`（`Transcript.tsx:294`）

缺的是归类函数和摘要函数，不需要改后端事件契约。

### 4.6 量级

`summarizeToolWork()` 约 60 行 + 折叠 UI 约 150 行。不涉及 runtime 改动。

---

## 5. 核心机制四：长会话不塌（分页 + 惰性渲染）

### 5.1 Kun 的解法

**5.1 轮次分页** —— `MessageTimeline.tsx`：

```ts
const TURN_PAGE_SIZE = 18
```

只渲染最近 18 轮，向上滚动增量加载更早轮次。

**5.2 busy 时强制收敛到一页** —— `use-timeline-scroll.ts` 的
`deriveTimelineRenderedTurnCount()`。源码注释写得很直白：

> 从展开的长会话发消息，曾经会先渲染整个历史一帧，再被 effect 收起。
> 那一瞬间的 Markdown 挂载足以把 Chromium renderer 打爆。

**这是真实事故的修复记录**，不是预防性设计。值得警惕：
"先渲染再收起"的一帧，在长会话下足以崩渲染进程。

**5.3 `useDeferredRender`** —— `src/renderer/src/hooks/use-deferred-render.ts`：

三级延迟策略，重内容（diff、代码块、图表）只在接近视口且浏览器空闲时才挂载：

```
IntersectionObserver(rootMargin: '300px')
  → debounce 300ms
  → requestIdleCallback(timeout: 500ms)
  → 才真正 setShouldRender(true)
```

滚动离开视口会 `clearPending()` 取消尚未触发的挂载。有 `IntersectionObserver` 缺失的
降级路径（直接渲染）。

**5.4 时间线整体懒加载** —— `LazyMessageTimeline.tsx` 用 `React.lazy` + `Suspense`
把整个时间线组件拆出首屏 bundle。

### 5.2 rove 现状

**全部未实现。**

- `Transcript.tsx:98` 是 `timeline.map()` 全量渲染，无分页、无虚拟化、无窗口化
- **风险叠加**：`Transcript.tsx:300` 的 tool 输出也走 `RichText` 全量 markdown 渲染。
  一个输出很大的 tool call 会和消息渲染叠加成双重成本
- `RichText` 有 30 万字符上限（`product-v2/RichText.tsx:17`）作为兜底，
  但这是**单条消息**的上限，不解决"很多条消息"的问题
- 已有的局部优化：`RichCodeBlock` 和 `MermaidDiagram` 用了 `next/dynamic`
  （`RichText.tsx:10-15`），方向对，但只是代码分割，不是视口惰性挂载

长会话下这是明确的性能墙。

### 5.3 量级

分页 + prepend 补偿约 150 行；`useDeferredRender` 约 120 行。

---

## 6. 核心机制五：流式专用的 Markdown 管线

### 6.1 Kun 的组合

| 用途 | 选型 |
|---|---|
| 流式 markdown | `streamdown` ^2.5.0（专为流式设计，处理未闭合语法） |
| 语法高亮 | `shiki` ^3.23.0（VS Code 同款 TextMate 语法） |
| 数学公式 | `@streamdown/math` + `katex` |
| HTML 安全 | `rehype-harden`（净化而非屏蔽）+ `rehype-raw` |
| GFM | `remark-gfm` |

### 6.2 最有价值的部分：他们踩的坑

`StreamdownAssistant.tsx` 里把 Streamdown **自带的流式能力全部关掉**：

```ts
<Streamdown
  mode="static"
  parseIncompleteMarkdown={false}
  isAnimating={false}
  animated={false}
  ...
```

源码注释给了两条具体原因：

1. **块修复会留下残留文本**：在含 GFM 表格的长回复里，Streamdown 的 block-repair 路径
   会在修复块旁边留下过期片段，DOM 里复制出来是 `"Work Workstreamstream"` 这种重复。

2. **自带动画与自己的 pacing 冲突**：一个 bursty chunk 横跨多个 markdown 块时，
   每个受影响的行同时 blur-in，"散落在各个 bullet 上的半透明补丁读起来像洞，不像打字"。

**结论：不要迷信流式 markdown 库的自动修复和自带动画，自己控制节奏更可靠。**
库用来做解析，节奏自己管。

### 6.3 另一个细节：key 策略

```ts
const streamdownKey = streaming ? 'live' : `static:${displayText.length}`
```

流式中固定为 `'live'`，保证打字机不会在中途被拆毁重挂载。
流式结束后切成 `static:${length}`，让后续任何编辑触发**干净重挂载**，
而不是依赖库的 block-diff 原地替换 —— 注释说后者在
"bullet → paragraph 且含 inline code"的转换上被观察到留下过期片段。

### 6.4 rove 现状

`apps/web/product-v2/RichText.tsx`：

- 裸 `react-markdown` ^10.1.0，每个 delta 全量重解析
- **无未闭合语法处理**：流式中途的 ``` 未收尾、表格画一半、`**` 单边，
  都会渲染成中间态并抖动
- 高亮用 `prism-react-renderer` ^2.4.1，保真度低于 shiki
- `skipHtml`（`RichText.tsx:26`）—— 直接丢弃 HTML 而非净化
- 图片完全屏蔽，替换成 `BlockedImage` 占位（`RichText.tsx:80-87`）
- 无数学公式支持

`skipHtml` 和屏蔽图片是**安全上保守但正确**的默认，不算缺陷。
但如果将来要支持模型输出图表/图片，需要的是净化管线（如 `rehype-harden`）而不是放开 `skipHtml`。

### 6.5 量级

替换 markdown 管线约 200 行改动 + 依赖变更。属于结构性改动，只有在当前
管线存在可复现的正确性或性能问题、且依赖审计通过后才进入实现，不因参考项目
采用它就默认批准。

---

## 7. 机制六：活动指示器用"内容本身在活动"

### 7.1 Kun 的 shiny-text

位置：`src/renderer/src/styles/base-shell.css:1186`

用 `background-clip: text` 加一条移动的渐变，做出"光扫过文字"：

```css
.ds-shiny-text {
  --shine-base: rgba(84, 103, 140, 0.95);
  --shine-peak: rgba(133, 193, 241, 1);
  --shine-size: 104px;
  background-image:
    linear-gradient(90deg, var(--shine-base) 0%, var(--shine-base) 27%,
      var(--shine-soft) 40%, var(--shine-peak) 50%, var(--shine-soft) 60%,
      var(--shine-base) 73%, var(--shine-base) 100%),
    linear-gradient(90deg, var(--shine-base) 0%, var(--shine-base) 100%);
  background-clip: text;
  -webkit-text-fill-color: transparent;
  animation: ds-shiny-text 2.05s linear infinite;
}
```

用法（`message-timeline-process.tsx:415`）：

```tsx
<span className={active && !hasRuntimeError ? 'ds-shiny-text' : ''}>{title}</span>
```

**设计要点：它标记的是内容本身在活动，不是在旁边转个圈。**
spinner 是"某处有个东西在忙"，shiny-text 是"这一行正在发生"。
后者语义更准确，视觉上也安静得多 —— 因为它不增加新的视觉元素。

注意 `!hasRuntimeError` 的条件：出错的行不发光。状态语义不混淆。

### 7.2 rove 现状

`apps/web/styles/` 全部 keyframes 只有两个：

- `product.css:991` `inspector-shimmer`
- `product-v2.css:843` `product-v2-pulse`

流式中的消息只有 `data-status="streaming"` 属性和 byline 上的 `"responding"` 文字
（`Transcript.tsx:164`）。**没有任何视觉上的"活着"的表达。**

### 7.3 量级

约 20 行 CSS。性价比极高。

---

## 8. 机制七：动效 token 化 + 完整 reduced-motion 覆盖

### 8.1 Kun

- **统一时长 token**：`--ds-duration-functional: 150ms`（`base-shell.css:24`）。
  功能性动效（hover、展开、focus）全部引用它，不散落魔法数字。
- **60+ keyframes，但每一处都配 `prefers-reduced-motion`**。
  `base-shell.css` 里 9 处 `@media (prefers-reduced-motion: reduce)`，
  `graph-workbench.css` 还有 1 处。模式统一：

```css
@media (prefers-reduced-motion: reduce) {
  .ds-subagent-mount,
  .ds-subagent-dot-pulse,
  .ds-subagent-lane-sweep {
    animation: none !important;
  }
}
```

- **进入动效有统一语汇**：`ds-subagent-mount` 用
  `opacity: 0 → 1` + `translateY(6px) → none`，`cubic-bezier(0.2, 0.7, 0.2, 1)`，0.3s。
  新卡片"浮上来"而不是"闪出来"。
- **交错延迟**：`animation-delay: var(--ds-subagent-stagger, 0s)`，
  多个同类元素靠 CSS 变量错开，不用 JS 算。

### 8.2 rove 现状

- **无动效时长 token**。`tokens.css` 里 `--radius-*`、`--shadow-*`、`--sidebar-width` 都有，
  唯独没有 duration / easing。
- `transition` / `animation` 出现次数：`product.css` 7 次、`product-v2.css` 10 次。
  动效语汇几乎为空。
- `prefers-reduced-motion` 只有 2 处（`product.css:1001`、`product-v2.css:2948`），
  覆盖那 2 个 keyframes。这个覆盖率本身是够的 —— 因为动效本来就少。

**这里的问题不是"reduced-motion 没做好"，而是"动效几乎不存在"。**
补动效时必须同步建立 token 和 reduced-motion 约定，否则会重复 Kun 那种散落 60 个
keyframes 的局面。

### 8.3 量级

token 定义约 10 行；随动效实现逐步铺开。

---

## 9. 机制八：设计 token 分两层

### 9.1 Kun 的两层结构

`base-shell.css:1-105`：

**原语层**（描述"是什么颜色"）：

```css
--bg-app: #ededed;
--surface-1: #f8f8f8;
--border-soft: #dcdfe3;
--text-primary: #3c3f43;
```

**语义层**（描述"用在哪"），引用原语层：

```css
--ds-bg-main: var(--bg-app);
--ds-surface-card: var(--surface-1);
--ds-border: var(--border-soft);
--ds-text: var(--text-primary);
```

组件只用 `--ds-*`。Tailwind config 也只映射 `--ds-*`（`tailwind.config.js`）。

**收益：换主题只改原语层，语义层和所有组件一行不动。**
`[data-theme='dark']` 块里只重定义了原语（`--bg-app: #2a2828` 等），
语义层的映射关系完全复用。

还用了 `color-mix()` 做派生色，减少手写值：

```css
--ds-sidebar-surface-chrome-bg: color-mix(in srgb, var(--ds-bg-sidebar) 94%, var(--ds-surface-card) 6%);
```

### 9.2 rove 现状

`apps/web/styles/tokens.css` 是**单层**结构。`:root` 和 `[data-theme="dark"]`
各自重复定义了全部 ~45 个 token：

```css
:root { --bg: #f4f5f3; --surface: #fbfcfb; --text: #1a1f1c; ... }
[data-theme="dark"] { --bg: #141816; --surface: #1e2320; --text: #e8ebe8; ... }
```

现在只有两个主题，重复量可控。但：

- 加第三个主题（如高对比度、纯黑 OLED）要再抄一遍全部 45 个值
- 无法表达"这个 surface 和那个 surface 用的是同一个底色"这种意图

`tokens.css` 顶部的注释约定值得保留：
"No pure #000 / #fff as design tokens; green/amber/red are semantic only"
—— 这条纪律 Kun 也遵守（最亮是 `#ffffff` 仅用于 `--surface-3`/hover，最暗 `#2a2828`）。

### 9.3 量级

重构约 60 行。建议在动效 token 化时一起做。

---

## 10. 机制九：用户可调的显示密度

### 10.1 Kun

`src/renderer/src/lib/apply-theme.ts` 暴露三个运行时可调项，直接写 `documentElement.style`：

```ts
export function applyUiFontScale(scale: UiFontScale): void {
  document.documentElement.style.setProperty('--ds-ui-scale', String(normalizeUiFontScale(scale)))
}

export function applyChatContentMaxWidth(widthPx: ChatContentMaxWidthPx): void {
  document.documentElement.style.setProperty('--ds-chat-content-max-width', `${...}px`)
}
```

- `--ds-ui-scale` —— 整体字号缩放
- `--ds-chat-content-max-width` —— 阅读宽度（长文阅读体验的关键项）
- `writeFontStackFor()` —— 写作模式字体栈

每个都有 `normalize*()` 做边界收敛，防非法值。
主题偏好支持 `'system' | 'light' | 'dark'`，`system` 会挂
`matchMedia('(prefers-color-scheme: dark)')` 监听并在卸载时正确移除。

**低成本高体感**：这类设置项让不同视力、不同屏幕尺寸的用户都能舒服使用，
而实现只是几个 CSS 变量。

### 10.2 rove 现状

`tokens.css` 有 `--sidebar-width: 280px`、`--inspector-width: 320px`、`--topbar-height: 52px`
等布局常量，但**全部是固定值，无运行时调节入口**。

主题：`tokens.css` 用 `[data-theme="dark"]` 选择器，且
`apps/web/platform/web.ts` 已有 `system` 偏好解析和 `matchMedia` 判断。本文
没有核实完整的监听、持久化和跨平台同步行为，因此这里的差距应写成“需要
验证并补齐”，不能写成“没有 system 支持”。

无字号缩放、无阅读宽度调节。

### 10.3 量级

约 40 行 + Settings UI。

---

## 11. rove Web 前端问题汇总

按性质分类。每条都标了位置。

### 11.1 确定的缺陷

| # | 问题 | 位置 | 影响 |
|---|---|---|---|
| D1 | 流式期间不自动滚动。effect 依赖 `itemCount`，但流式只改 `content` 不改条目数 | `chat/Transcript.tsx:46-60` | 长回复时文字长出视口，用户看不到正在生成的内容 |

### 11.2 体验缺失

| # | 问题 | 位置 | 说明 |
|---|---|---|---|
| E1 | 无 typewriter pacing，文本按 SSE 分块成块出现 | `chat/Transcript.tsx:167` | 见第 2 节 |
| E2 | 无活动指示器，流式中只有 `"responding"` 文字 | `chat/Transcript.tsx:164` | 见第 7 节 |
| E3 | 动效语汇近乎空白（全站 2 个 keyframes，17 处 transition） | `styles/product*.css` | 见第 8 节 |
| E4 | 无字号缩放 / 阅读宽度调节 | `styles/tokens.css` | 见第 10 节 |
| E5 | 流式中途未闭合 markdown 语法会抖动 | `product-v2/RichText.tsx` | 见第 6 节 |

### 11.3 信息架构问题

| # | 问题 | 位置 | 说明 |
|---|---|---|---|
| A1 | tool call 平铺，N 步 = N 张卡片，无归类无摘要 | `chat/Transcript.tsx:288` | 见第 4 节。信息密度差距最大处 |
| A2 | `eventSeq` 等开发者调试信息出现在产品主界面 | `chat/Transcript.tsx:210-213` | 应移入 Inspector |
| A3 | byline 显示 `"canonical message"` 这类内部术语 | `chat/Transcript.tsx:164` | 面向 runtime 概念而非用户概念 |
| A4 | 运行分组标题用 `"Turn N"` + 裸 run id | `chat/Transcript.tsx:105-116` | 缺少"这一轮做了什么"的概括 |
| A5 | Composer 要求用户预先选择 Steer 或 Follow-up | `chat/Composer.tsx:76-163` | 技术实现模式直接暴露给用户；替代契约不由本文定义，见 2026-08-10 productization 文档 |

### 11.4 性能风险

| # | 问题 | 位置 | 说明 |
|---|---|---|---|
| P1 | 时间线全量渲染，无分页/虚拟化 | `chat/Transcript.tsx:98` | 见第 5 节。长会话性能墙 |
| P2 | tool 输出也走全量 markdown 渲染，与消息渲染成本叠加 | `chat/Transcript.tsx:300` | 大输出 tool call 是双重风险 |
| P3 | 重内容无视口惰性挂载（`next/dynamic` 只做了代码分割） | `product-v2/RichText.tsx:10-15` | 见第 5.3 |

### 11.5 工程卫生

| # | 问题 | 位置 | 说明 |
|---|---|---|---|
| H1 | `product-v2.css` 2963 行、`product.css` 2243 行，文件职责偏重 | `styles/` | 可维护性风险；历史 Archive 中有 800 行建议，但当前 AGENTS.md 未将其规定为硬性门槛 |
| H2 | 单层设计 token，加主题要全量重抄 | `styles/tokens.css` | 见第 9 节 |
| H3 | 无 i18n，全英文硬编码 | `apps/web/` 全体 | 已确认未做。注：文案硬编码会让后续接 i18n 的改动面很大 |

### 11.6 已做对的地方（不要在改造中丢掉）

- **`RichText` 的安全默认**：`skipHtml`、`safeRichTextUrl()` 白名单
  （只放行 `#`、单斜杠相对路径、`https?:`、`mailto:`）、外链强制
  `rel="noreferrer noopener"`、30 万字符上限 —— `product-v2/RichText.tsx:63-101`
- **可访问性基础**：transcript 上 `role="log"` + `aria-live="polite"` +
  `aria-relevant="additions text"`；tool card 展开用 `aria-controls` / `aria-expanded`
  —— `chat/Transcript.tsx:84-88, 297-299`。**Kun 在这方面并不比我们好。**
- **`RestoreNotice` 的诚实降级**：区分 loading / partial / error，
  partial 会逐条列出 `describeTranscriptPartialReason()`，
  error 明确写 "No empty history has been substituted for the failed read."
  —— `chat/Transcript.tsx:255-259`。这条契合仓库"不把失败伪装成成功"的纪律，
  **比 Kun 的处理更严谨**，改造中必须保留。
- **`atLatest` 阈值判定与 "Return to latest" 按钮**逻辑正确（`Transcript.tsx:66-78`），
  只需补内容依赖。

---

## 12. 借鉴优先级

本节只排序表现层收益，不代表整个产品化程序的全局优先级。Provider 首次配置、
真实 provider 验证、Agent 成功率和恢复能力由综合产品化计划排序，并可能先于
本节中的动画或 Markdown 工作。

### 第一梯队 —— 几百行内，体感提升最大

| 顺序 | 项 | 对应问题 | 量级 |
|---|---|---|---|
| 1 | 修流式自动滚动；抽 `useTranscriptScroll` hook | D1 | 10 行（修）/ 120 行（抽） |
| 2 | Typewriter pacing hook（含 grapheme 边界） | E1 | ~60 行 |
| 3 | 过程折叠 + 工作摘要；`eventSeq` 移入 Inspector | A1, A2 | ~210 行 |
| 4 | 进行中行的 shiny-text 指示器 | E2 | ~20 行 CSS |

这四项全部在 `apps/web/chat/` 与 `apps/web/styles/` 内，不触碰 runtime、
不改事件契约、不动 API。改动面可控，可独立验证。

### 第二梯队 —— 结构性

| 顺序 | 项 | 对应问题 |
|---|---|---|
| 5 | 轮次分页 + prepend 位置补偿 + `useDeferredRender` | P1, P2, P3 |
| 6 | `streamdown` 替换裸 react-markdown（关掉库自带动画与块修复） | E5 |
| 7 | `shiki` 替换 `prism-react-renderer` | —— |
| 8 | token 分两层 + 动效时长 token 化 + reduced-motion 约定 | H2, E3 |
| 9 | `styles/` 拆分到 800 行以下 | H1 |

### 第三梯队 —— 产品化

| 顺序 | 项 | 说明 |
|---|---|---|
| 10 | **Provider 预设目录** | 见第 13 节。这是 provider 领域我们唯一的真实差距 |
| 11 | 字号缩放 / 阅读宽度 | E4 |
| 12 | i18n | H3。你自己用中文，实际优先级可能高于此处排序 |
| 13 | 桌面发布体验 | full-delivery 假设已包含 Desktop host；剩余是安装、更新、签名、崩溃和平台证据，不是“是否采用 Tauri”的架构选择 |
| 14 | 从 Rust 生成 TS 类型 | 见第 13 节 |

---

## 13. 架构层面的两处订正与一处真实差距

### 13.1 订正：Provider 可扩展性，rove 领先（不是落后）

早期分析曾把 provider 扩展列为 rove 的短板，**该判断基于一份 2026-07-23 旧分支的
过期笔记，结论是错的**。当前 `main` 已完成 provider 层重构：

| 能力 | rove 现状 | 位置 |
|---|---|---|
| 协议 ID | **开放字符串**，非闭合 enum。注释明确："deliberately not represented by a closed enum… applications may register additional canonical IDs" | `models/src/provider/id.rs:20` |
| 运行时注册 | `WireProtocolRegistry::register()`，装配期可注册任意协议 | `models/src/provider/registry.rs` |
| 免重编译接新格式 | `external-adapter-v1`：JSONL over stdin/stdout 外部进程适配器。注释："so unsupported wire formats can be handled without recompiling Rove" | `models/src/provider/external_adapter.rs:1-5` |
| 自定义认证 / header | `AuthStyle`、`ResolvedAuth`、`ResolvedHeader` | `models/src/provider/auth.rs` |
| 工厂分发 | 按 `protocol_id` 查 registry，已非 match 硬编码 | `apps/bootstrap/src/factory.rs:109-123` |

而 Kun 的 `@kun/provider-catalog` 是 **577 行纯数据预设表**，其
`ProviderCatalogEndpointFormat` 反而是 4 值**闭合**联合：
`'chat_completions' | 'responses' | 'messages' | 'custom_endpoint'`。

**架构上我们更开放。**

### 13.2 真实差距：预设目录 / 开箱引导

Kun 有而我们没有的，是预设表本身承载的**上手体验**：

```ts
type ProviderCatalogPreset = {
  id, name, category, kind, authFlow, authType,
  baseUrl,           // 自动填好
  endpointFormat,
  models,            // 模型列表直接给
  docsUrl,           // 文档在哪
  credentialUrl,     // 去哪拿 key
  tokenPlan?: { baseUrl, regions, models, credentialUrl }
}
```

用户选"Claude"，base URL、可用模型、拿 key 的链接全部自动就位，
不需要读文档、不需要手填 endpoint。还区分 `api` / `subscription` 两类，
支持 OAuth 与订阅登录流程，`tokenPlan` 里连区域端点都列了。

**这是配置体验层的差距，不是架构层的差距，量级也小得多** ——
本质是一张数据表加上 Settings 里的选择器 UI。

### 13.3 类型共享：单语言红利 vs 跨语言成本

Kun 的 GUI 直接 `import` runtime 的 `@shared/*` 类型，白拿类型安全。
我们跨 Rust/TypeScript 边界，`apps/web/lib/rove-types.ts` 是手工对齐的
（有 `rove-types.test.ts` 守着，但仍是人工维护）。

既然已经有 OpenAPI surface，`ts-rs` 或 schemars → OpenAPI → codegen
可以把这份手工成本消掉。**本文未核实是否已有 codegen 流程**，需要单独确认。

---

## 14. 不要学的

### 14.1 文件规模失控

| 文件 | 行数 |
|---|---|
| `src/renderer/src/styles/base-shell.css` | 5694 |
| `src/renderer/src/styles/surfaces-write.css` | 5022 |
| `src/renderer/src/components/chat/message-timeline-bubbles.tsx` | 2302 |
| `src/renderer/src/components/chat/FloatingComposer.tsx` | 2065 |
| `src/renderer/src/components/chat/message-timeline-process.tsx` | 1835 |
| `src/renderer/src/components/chat/MessageTimeline.tsx` | 1368 |

`src/renderer` 共 1383 个文件，`components/` 单目录 555 个。
文件规模明显增加维护成本。历史 Archive 文档曾提出 800 行上限，但当前
`AGENTS.md` 没有把 800 行/文件、50 行/函数规定为现行硬门禁，因此应按职责、
测试成本和变更冲突判断拆分，而不是机械按行数验收。

**但要公道**：我们 `product-v2.css` 2963 行同样超标（见 H1）。
这条是双向的，不能只当作对方的问题。

### 14.2 吉祥物动画的投入产出

60+ keyframes 中相当一部分是吉祥物动画：`ds-ikun-*` 与 `ds-work-logo-*` 系列
（跑动、跳水、冲浪、喝奶茶、探头、扭动……），加上 `AnimatedWorkLogo.tsx` 480 行，
CSS 部分接近 2000 行。

这是品牌人格投入，和"专业工具的可信感"不完全同向。按产品定位取舍 ——
**我们要学的是第 2-10 节那些机制，不是这一部分。**

### 14.3 一处理念差异

Kun 的 `RestoreNotice` 等价物在处理历史恢复失败时，不如我们明确。
我们 `Transcript.tsx:258` 显式写出
"No empty history has been substituted for the failed read."
—— 这条纪律不要因为学习对方的 UI 而丢掉。

---

## 15. rove 不该丢的优势

改造表现层时，以下是必须保留的：

- **Rust 边界**：工作区路径边界、provider payload 隔离在 model 层之后、
  `ToolRegistry` 统一审批路径
- **持久化三分职责**：`trace.jsonl` 记事件事实 / `task_state.json` 记可恢复状态 /
  `report.json` 是派生摘要。不把 report 当唯一真相
- **fail-closed 迁移**、**确定性 fake-provider benchmark**、**无 key 无网络可跑**
- **`AGENTS.md` 纪律**：文档必须标注是否已实现；不静默选择代码/文档矛盾的一边

Kun 在 README 里承诺了很多这类性质的东西，我们是在 CI 里强制执行。
**表现层可以补，这套纪律很难反向补。**

---

## 16. 核实方式与未核实项

### 已核实

- Kun：浅克隆 `master`（161MB），读了 `src/renderer/src/components/chat/`、
  `styles/base-shell.css`、`hooks/use-deferred-render.ts`、`lib/apply-theme.ts`、
  `packages/provider-catalog/src/index.ts`、`tailwind.config.js`、`package.json`、`kun/src/` 目录结构
- rove：读了 `apps/web/chat/`、`apps/web/product-v2/`、`apps/web/styles/`、
  `models/src/provider/`、`apps/bootstrap/src/factory.rs`

### 已补充核实

- rove 的 `apps/web/platform/web.ts` 已支持 `'system'` 偏好并通过
  `matchMedia('(prefers-color-scheme: dark)')` 解析当前系统主题；是否持续监听
  系统主题变化仍应由实现和测试确认。

### 未核实（后续如需可继续）

- rove 是否已有 Rust → TS codegen 流程
- `streamdown` / `shiki` / `rehype-harden` 各自的许可证条款
- Kun 的 Agent Graph 权限约束实现、`.kunx` 扩展沙盒机制
- Kun 的 SSE 事件契约与我们 canonical events 的逐项对照

### 复现环境

Kun 浅克隆位于 `/tmp/kun-analysis`（临时目录，重启后失效）。
需要重新核实时：`git clone --depth 1 https://github.com/KunAgent/Kun.git`

---

## 17. 审计结论交接（非实施入口）

在假设 full-delivery 已完成的基线上，本文结论按以下方式进入产品化程序：

| 分类 | 项目 | 决策 |
|---|---|---|
| 已确认缺陷 | 流式内容增长时自动滚动不触发 | 直接进入产品体验工作流并补浏览器回归 |
| 高价值信息架构 | tool 过程折叠、摘要、隐藏内部 ID | 优先于装饰性动画 |
| 性能基础 | 长会话分页/窗口化、重内容惰性渲染 | 在长会话基准下实现和验收 |
| 待测假设 | typewriter pacing、shiny text | 先测 CPU、可访问性、积压和 reduced-motion |
| 待审依赖 | Streamdown、Shiki、HTML/数学插件 | 完成许可证、安全、bundle 和正确性评估后再决定 |
| 产品入口 | Provider preset、首次配置、连接测试 | 纳入综合产品化 P0，不作为纯前端美化任务 |
| 控制入口 | 当前 Composer 直接暴露 Steer/Follow-up 技术模式 | 事实已交给总计划 Workstream F；本文不定义替代契约或验收标准 |
| 已被基础交付吸收 | Tauri 架构选择 | 不再重复设计；关注安装、签名、更新和平台证据 |

### 17.1 Interaction amendment (2026-08-10)

The explicit composer modes described in the 2026-07-27 Product UI V2 design
are a historical implementation baseline. This reference only records the
current-surface contradiction. It does not define the replacement message
lifecycle, Runtime authority, migration, or acceptance criteria. Those future
requirements exist exclusively in
[`2026-08-10-post-full-delivery-productization.md`](../plans/2026-08-10-post-full-delivery-productization.md).
