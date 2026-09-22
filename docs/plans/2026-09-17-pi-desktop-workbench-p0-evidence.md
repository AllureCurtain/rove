# PI-Desktop 工作台 P0 阶段性证据

> 状态：**历史 P0 实机验证仍 Blocked / Incomplete；经用户调整为源码参考后，P1a 首批及 P1b 键盘切片已实现，整体设计部分实现**。最新进展见第 7–8 节；第 1–6 节保留历史记录。
> 本报告记录本轮实际尝试，不复用旧记录中的测试通过结论。
> 关联：[设计](../design/2026-09-17-pi-desktop-workbench-design.md)、[实施门禁](2026-09-17-pi-desktop-workbench-implementation.md)。

## 1. 基线与写入边界

- 目标 worktree 为 `.worktrees/workbench-p1a`，分支 `feature/pi-desktop-workbench-p1a`。
- 本轮核实 HEAD 为 `7e54ab6af1158e0eb2a7418e67708999db25da4d`。
- 开始时仅两份任务文档未跟踪，无产品代码修改；本轮保留它们。
- 未 fetch/pull，不能宣称与当前远端同步。
- PI 参考源码本轮核实为 `8d826433`，`git status --short` 无输出。未修改、安装依赖或在参考目录构建。
- 未修改主工作区或旧 worktree；未复制旧 WorkspaceTree.tsx 修改。
- 临时源码、脚本、数据和日志均放在会话 scratch 的 `pi-p0-launch/`，不提交。

## 2. 实际启动尝试

平台为 Windows；本轮命令观察到 Node `v24.9.0`，当前目录外 pnpm `10.30.3`。PI 根 package.json 要求 pnpm `11.18.0`。

1. 阅读 PI `scripts/dev-electron.mjs` 与 `scripts/e2e-electron-boot.mjs`。前者通过 electron-vite 启动，后者要求预先构建桌面与 host，并使用 `PI_DESKTOP_DATA_DIR` 隔离数据。直接运行参考目录的 `pnpm dev` 会触发构建写入，因此不采用。
2. 用 `git archive` 导出 `8d826433` 到 scratch，再解压为独立源码副本。导出命令退出码 0。
3. 在该副本设置独立 `PI_DESKTOP_DATA_DIR`、TEMP/TMP，清除 renderer URL 和 ELECTRON_RUN_AS_NODE 后执行 `pnpm dev`，工具设定 60 秒超时。结果为 **TOOL_TIMEOUT**，不是应用退出码，也不是启动通过。
4. 日志仅观察到 Corepack 将下载 `https://registry.npmjs.org/pnpm/-/pnpm-11.18.0.tgz` 的提示，没有进入 Electron 的证据。不能据此确定是网络不可用、交互等待还是其他下载问题。PowerShell 输出为 UTF-16；文本工具拒读后按 Unicode 编码读取成功。
5. 编写并执行 scratch 中的 `direct-launch.cjs`，绕过 pnpm 做直接启动前置检查。实际退出码 **2**，`electronError = MODULE_NOT_FOUND`，`electronBinary = false`、`mainBuild = false`、`hostBuild = false`、`launched = false`。这是前置检查失败，不是 Electron 进程崩溃。
6. 超时后按命令行包含隔离路径查询进程，没有匹配残留；该检查不能证明任何无此路径的进程均不存在。未触碰用户正在使用的实例或凭据。
7. 最后只读检查参考目录的标准预构建路径：`apps/desktop/node_modules/electron/dist/electron.exe` 和 `apps/desktop/out/main/index.js` 均不存在，检查命令退出码 **2**，未启动进程。此结论只覆盖这两个明确路径，不断言机器上没有其他安装版；没有使用其他安装版代替同提交证据。

原始记录在本会话 scratch 中：`pi-p0-launch/launch.log`、`direct-launch.cjs`、`direct-launch.json`。这些不是随仓库持久交付的证据文件；关键输出和限制已在上文记录。

## 3. 六条旅程状态

共同测试目标为 PI `8d826433` / Windows / 独立测试数据。应用未启动，未创建可供点击验收的合成项目，也没有截图。

| 旅程 | 本轮直接交互 | 截图 | 结论 |
|---|---|---|---|
| 左栏项目、会话、置顶、菜单、折叠与设置 | 未运行 | 无 | 未验证 |
| 输入、快捷键、IME、发送与停止 | 未运行 | 无 | 未验证 |
| 审批请求、允许、拒绝与过时状态 | 未运行 | 无 | 未验证 |
| 目录展开、文件预览与失败态 | 未运行 | 无 | 未验证 |
| 会话变更与 Git 差异 | 未运行 | 无 | 未验证 |
| 外链、本地预览与浏览器失败态 | 未运行 | 无 | 未验证 |

根 LICENSE 确认为 LGPL v3 文本，开头声明其纳入 GPL v3 条款并附加许可。第三方依赖与素材条款尚未完成逐项核实；不能将根许可证解释为所有资产均可复制。建议独立实现交互思路，不复制源码、品牌或素材。

## 4. rove 当前能力映射：文档级，源码级未完成

已读 [ONBOARDING](../ONBOARDING.md)、[Runtime 索引](../runtime/README.md) 和 [subsystems](../runtime/subsystems.md) 的 API/Web 章节。下表是当前状态文档明确记载的合同，不是本轮执行测试证明；完整函数、字段、分页上限及组件所有者仍需源码核实。

| 工作台需求 | 现有合同及证据 | 实施约束 / 待核实 |
|---|---|---|
| 左栏工作区与会话 | subsystems.md 733–769 行：ProductApp、ProductStore、`/w/:workspaceId`、`/w/:workspaceId/s/:sessionId`、`/settings/:section` | 复用服务端目录和路由；置顶字段是否存在尚未核实，不能宣称缺失或放假按钮 |
| 前台与后台状态 | 771–780 行：聚焦 job 独占 EventSource，后台状态由有界目录轮询刷新 | 导航切换不取消执行；具体状态枚举、轮询频率待核实 |
| 输入及队列 | 822–831 行：统一 Send Message、服务端 FIFO、promote/revoke、独立 Stop | 不新建前端队列；草稿键与异步提交边界待核实 |
| 当前审批 | 675–682 行：`GET /jobs/{job_id}/state`、`GET /jobs/{job_id}/events`、`POST /jobs/{job_id}/approvals/{call_id}`；760–766 行记载共享 reducer 的审批投影 | 右栏复用现有权威；请求 payload、并发校验、历史字段待核实；Web 经 `/api/*` 代理 |
| 文件、Artifact、图片、diff | 847–855 行明确已有有界、工作区/会话 scoped 的产品能力 | 不是全部缺失；具体端点、ignore、分页和字节限制待源码核实 |
| Review 与证据 | Runtime README 36–45 行明确 hard read-only Review、不可变 Git 目标与 API/CLI/Web 投影 | Review 不等于工具权限审批，不替换已有工作流 |
| Desktop 与浏览器 | Runtime README 10–15 行明确 Tauri D0 复用 API router/ProductStore | 不能据此认定已有任意网页浏览器或隔离 HTML 预览；外链宿主入口待核实 |

探索子代理曾启动广泛源码映射，但在本轮收敛时被停止，未取得完整最终报告。其未返回的中间调查不作为已交付证据。**P0 的完整 API 字段对照尚未完成。**

## 5. 门禁与建议缩减范围

P0 退出条件未满足，不进入 P1a，不更新 Runtime 完成状态。

供用户选择：

1. **继续完整 P0**：只在 scratch 安装指定依赖并构建同提交 PI，显式处理 Corepack 非交互与下载问题；完成六条旅程、许可证边界和源码字段映射，再确认交互。不使用用户凭据或现有会话，不运行真实 Provider 测试。
2. **建议缩减**：经用户明确同意，豁免本次 PI 六项实机作为首批前置条件，但保留“未验证”标识。先补齐仅 P1a 涉及的 rove 源码/字段映射，再做左栏呈现与现有导航操作；首批不扩展 schema、不做置顶持久化、不启用尚未交付的文件/证据快捷入口，不启动 P1b/P2–P5。置顶是否延期及其他 P1a 验收项变化需同步修订计划，不能静默削减。

已确认的三栏和左栏参考方向不等于上述豁免授权。用户确认前，本轮只交付本报告，不修改产品代码。

## 6. 验证范围

- 本轮未运行 Rust/Web/PI 测试、未做真实点击、未获得截图；旧记录的测试通过不算本轮结果。
- 两次启动路径均没有产生实机通过证据。
- 不将源码检查、mock、live API 或安装版验收混为同一种证据。
- 本报告为文档变更，交付前检查相对链接、围栏、标题、尾随空白及 Git diff/status；不为文档单独运行完整产品测试。

## 7. 后续调整与 P1a 首批实现（同日追加）

用户已确认调整 P0 门禁方向：以 PI-Desktop 同提交源码（`8d826433`）为主要交互参考直接实施，不再把 PI 实机启动作为阻塞。第 2 节记录的启动 blocker 保留原样；第 3 节六条 PI 实机旅程**保持“未验证”**，不因方向调整改写为通过。

本节记录的 P1a 首批实现均在本 worktree 完成，全部为呈现层改动，无 API/schema/Runtime 变更：

- `apps/web/sidebar/WorkspaceTree.tsx`：项目展开与激活分离（独立 disclosure，点击不导航；激活项目自动展开，轮询不回滚手动折叠）；顶部常驻“新会话”绑定当前工作区，无工作区时打开现有工作区选择器而非创建孤儿会话；项目级操作菜单（固定/取消固定/移除）带 Escape、焦点归还、方向键和 `scrollIntoView` 防底部裁切；搜索范围说明与“无匹配”状态区分空工作区；父会话存在性按全量会话列表判定，避免搜索时误报 Fork 谱系丢失。
- `apps/web/styles/product-v2.css`：左栏紧凑行高、顶部新建按钮、搜索说明、菜单浮层样式。
- `apps/web/copy/zh-CN.ts`、`apps/web/copy/en-US.ts`：新增文案（中英对齐）。
- 新增 `apps/web/tests/e2e/workbench-navigation.spec.ts`：折叠不改路由、跨项目展开不改路由、菜单键盘/固定/不导航、搜索无匹配恢复、新建请求路径与 `workspace_id` 载荷、无工作区入口走选择器且焦点归还。

本轮真实验证（均在本 worktree、真实退出码）：`pnpm typecheck` 退出码 0；`pnpm exec vitest run` 43 个文件 318 个用例全部通过；`pnpm exec playwright test workbench-navigation.spec.ts shell.spec.ts` 8 条通过；`pnpm build` 退出码 0。独立只读审查提出的两处问题（菜单裁切、父会话误报）已修复并包含在上述验证内。

未做且不在首批：会话级置顶（服务端无字段，不做假按钮）、右栏面板、输入框改造、文件/证据入口、P1b–P5。旧 worktree 的 WorkspaceTree.tsx 修改未查看、未合并。

## 8. P1b 输入键盘切片（后续追加）

已实现，仅限呈现层，不改变 Runtime、API、ProductStore 或消息队列权限：

- `apps/web/chat/Composer.tsx` 复用统一提交入口，Ctrl/Meta+Enter 发送，Enter 与 Shift+Enter 保留换行；`isComposing` 与 `keyCode === 229` 阻止输入法确认误发送，空白或禁用状态不发送，成功接受后才清空草稿。
- 中英文 placeholder 与无障碍快捷键说明已同步；`Composer.test.tsx` 保留停止按钮断言并增加快捷键/文案断言。
- 新增 `workbench-composer.spec.ts` 的 4 条浏览器用例，覆盖两种发送组合键、成功清空、换行、空白和两类合成 IME 事件。合成 IME 事件不是操作系统输入法实机验收。
- 当前合同同步至 [subsystems](../runtime/subsystems.md)。

本轮测试子代理实际执行结果：`pnpm typecheck` 退出码 0；`pnpm exec vitest run` 退出码 0，43 文件 / 318 用例通过；Composer、navigation、shell 三个 Playwright 文件退出码 0，12 条通过（40.7 秒）；`pnpm build` 退出码 0。新增静态断言后另跑 `pnpm exec vitest run chat/Composer.test.tsx`，退出码 0。浏览器端口 13159，无外部 Provider，未启动 PI；构建生成的 `next-env.d.ts` 指针恢复原状。

独立只读审查未确认实现缺陷；指出失败保留、禁用非空/提交中防重复、组件回调直接验证 trim 的测试仍需补充。本切片不宣称覆盖这些边界的新增行为测试。原有逻辑保留，未添加新依赖或修改锁文件。

## 9. P1b 草稿隔离切片（后续追加）

已实现，仍仅限呈现层，无 API/schema/Runtime 变更、无新依赖、不使用浏览器存储：

- 先用真实浏览器回归暴露缺陷：切换会话再返回时草稿丢失（`workbench-composer.spec.ts` 首跑失败，输入框为空）。根因为草稿仅存于 Composer 组件本地 state，随会话切换卸载。
- 修复：新增 `apps/web/state/composer-draft-store.ts`（按 `workspace_id + product_session_id` 键控的内存草稿库，不可变版本快照，接受回执仅清除捕获版本，防 ABA；同步提交锁防重复事件与挂载竞态；`onSend` 抛错时保留草稿并显示通用文案，不自动重发）与 `apps/web/state/use-composer-draft.ts`（React 订阅 hook，独立渲染保持组件本地兼容）。
- `shell/ProductApp.tsx` 在持久产品布局持有 store 并显式传入会话身份；store 随布局存活，客户端路由、设置页往返均保留草稿；刷新/关闭页面即丢弃，不写 localStorage/sessionStorage。
- `chat/Composer.tsx` 接入 store、忽略 `event.repeat` 长按；中英文新增 `sendUnexpectedError` 通用文案。`onSend` 明确拒绝（返回 false）时错误仍由现有 continuity 显示，通用文案仅覆盖真抛错路径。
- 新增 `state/composer-draft-store.test.ts` 12 条 node 级用例（身份隔离、版本、ABA、异步锁、重复提交、失败保留、不驱逐）；`workbench-composer.spec.ts` 增至 8 条浏览器用例（会话往返草稿隔离、明确拒绝后原样保留并允许显式重试、提交中跨会话草稿互不干扰、长按 repeat 不发送）。
- 当前合同同步至 [subsystems](../runtime/subsystems.md)。

真实验证（本 worktree，真实退出码）：修复前目标用例实际失败（`toHaveValue` 收到空串）；修复后 `pnpm exec vitest run` 退出码 0（44 文件 / 330 用例），Composer + navigation + shell Playwright 16 条通过（33.9 秒），`pnpm build` 退出码 0；`pnpm exec tsc --noEmit` 退出码 0；草稿库专项 12 条通过。浏览器端口 13161/13162，无外部 Provider，未启动 PI；构建生成的 `next-env.d.ts` 指针恢复原状。

独立只读审查确认核心机制可靠（碰撞安全键、不可变快照、同步锁、版本 ABA 保护、双语错误文案、无持久化/单例 hydration 风险），并留下三条 P2 未在本切片处理：意外发送错误文案会遮蔽后续 continuity 错误直到编辑或重试；超时/结果不明路径尚无浏览器级用例（计划 83 行的超时退出条件依赖现有 continuity 重试逻辑，未新增直接覆盖）；busy 会话中 Stop 的浏览器级回归未加入本套件。这些留给后续切片，不宣称已完成。

第 3 节的六条 PI 实机旅程仍未验证。全部变更保留在当前 worktree，未暂存或提交，未修改主工作区、旧 worktree 或 PI 参考仓库；未新增依赖、未修改锁文件。

## 10. 全量 e2e 首跑的旧套件失败分诊（后续追加）

本切片首次运行全量 `pnpm test:e2e`（含此前未运行过的 continuity.spec.ts 与 polish.spec.ts），暴露 3 条失败。重跑定位（CI=true，端口 13172，真实退出码 1）：64 通过、3 失败、5 跳过（1.5 分钟）。逐条归因：

- continuity.spec.ts:209：旧测试直接点击 `从列表移除工作区` 按钮；P1a 将移除入口移入项目操作菜单后，该按钮不再直接可见，点击超时（页面快照确认菜单按钮 `rove-shell-demo 的操作` 在场）。属测试选择器过时。
- polish.spec.ts:79：断言 `getByRole("dialog", { name: "Workspaces" })`，但 P1a 侧栏的 aria-label 为本地化文案 `工作区`；快照确认 `role="dialog"` 与 `aria-modal` 实际存在。属断言字符串过时，语义未回退。
- 同步更新 polish.spec.ts:91 的 Tab 回绕预期：P1a 头部首个可聚焦按钮从 `添加工作区` 变为 `新会话`，保留从末项回绕到首项的焦点陷阱检查。
- polish.spec.ts:185：关闭抽屉的点击超时，同因抽屉名称定位失败所致；修正定位后，关闭、焦点陷阱与回焦断言全部通过。

修正仅限两个测试文件（产品代码零改动）：continuity.spec.ts:209-210 先点击 `${workspace.display_name} 的操作` 再点击 `从列表移除工作区` menuitem，设置按钮改为 banner 内定位（消除两义性）；polish.spec.ts:78/156 抽屉定位改为 `工作区`（exact）；polish.spec.ts:91 回绕预期改为 `新会话`。删除竞态（workspaceDeleteDelayMs 300 + poll + 450ms 等待）、抽屉角色/aria-modal、焦点陷阱与回焦断言全部保留，未削弱覆盖。

验证（真实退出码）：聚焦 3 用例重跑退出码 0（21.6 秒）；continuity + polish 全文件 22 条通过（46.9 秒）；`pnpm test --run` 44 文件 330 用例通过（8.2 秒，退出码 0）。最终由主会话直接运行 `pnpm test:e2e`，退出码 0，67 条通过、5 条跳过（1.3 分钟）；`pnpm typecheck` 退出码 0。最终浏览器端口 13174。跳过项不作为通过证据。本轮只修改测试和文档，未重跑此前草稿切片已通过的生产构建，也未运行 Rust 或外部服务门禁。

baseline 交叉验证说明：scratch 基线 worktree（HEAD 7e54ab6）原样复测时，Turbopack 拒绝指向 worktree 外部的 node_modules junction，webServer 未启动、用例未执行。随后主会话使用 `next dev --webpack` 独立服务器在端口 13175 做有界复测，90 秒仍未就绪，日志报 `.next/dev/fallback-build-manifest.json` ENOENT；测试未执行，已终止本次服务器。因此基线浏览器对照仍受环境阻塞，不宣称三项为基线已有失败；当前证据支持 P1a 呈现变化后旧测试契约未同步，修正后当前分支全量通过。独立只读审查子代理因配额限制中断，后续子代理记录不可收取；主会话自行复核最终 diff，未删除原有竞态、焦点和路由断言。

收尾：临时 baseline worktree 与 node_modules junction 已清理，依赖链接目标未删除；验证日志保留在 session scratch。`next-env.d.ts` 无净变更，`git diff --check` 通过。当前 worktree 保留 10 个已跟踪修改文件与 8 个未跟踪交付文件（含原有三份文档），未暂存、未提交；主工作区、旧 worktree、PI 参考仓库和锁文件均未改动。

## 11. P1b 验收补全（后续追加）

针对第 9 节遗留的三条 P2 项与验收矩阵 U2，本轮补齐：

- 错误优先级缺陷（P2 遗留第 1 条）已修复：`Composer.tsx` 展示逻辑改为当前 continuity 错误优先、`sendUnexpectedError` 仅作回退。先以真实失败红测确认缺陷（`chat/Composer.test.tsx` 新用例，输入框存在时旧发送失败文案遮蔽当前连接错误），修复后该文件 2 条用例、草稿库 12 条用例通过（退出码 0）。
- busy 停止浏览器用例已加入 `workbench-composer.spec.ts`：approval 模式发送后停止按钮可见、输入框禁用，点击停止后按钮消失且输入框恢复可用（断言用户可见结果；mock 内部 job 状态不作为断言对象）。该文件现为 9 条用例全部通过（23.9 秒，退出码 0）。
- 超时/结果不明路径：`continuity.spec.ts` 既有用例 "a committed job survives a disconnected response and delayed binding visibility" 实跑通过（11.9 秒，退出码 0），覆盖断线后保守确认与绑定恢复；composer 侧失败保留与显式重试已有专项用例。

一项新增的幂等键重试实验用例因夹具缺陷在验证前主动撤回，不影响上述结论；未修改产品发送协议。

## 12. P2 单点回归修复与后续边界

- `shell.spec.ts` 的审批完成断言原先在整个 Conversation 查找同一文本。真实 Chromium trace 显示工具结果与助手最终回复各呈现一次，宽泛 locator 触发 strict-mode；不是两条助手回复。仅将断言限定到助手 article，并额外要求恰好一条，未使用 `.first()` 隐藏重复。
- 修复前 `shell.spec.ts -g 'inline approval' --trace on` 退出码 1。修复后 `shell.spec.ts workbench-panel.spec.ts` 共 14 条通过，26.1 秒，退出码 0。trace 和日志为本地生成证据，不提交。
- P2 请求仍使用既有审批 API，不增加服务端 revision/CAS 字段。消息与面板共享 controller；客户端绑定检查、重复提交锁和有限权威重读不构成新的授权引擎。
- 本轮收敛到单点修复，不据此宣称 P3–P6 完成。后续源码调查确认：现有文件浏览 handler 没有证明 ProjectTrust capability 检查已覆盖该链路；P3 的信任退出条件仍未满足。P4 的 canonical 事件可证明请求与执行结果，但没有权威决策 ID、actor 或决策时间，不能从工具完成推断用户授权。
- P5a 已有 Desktop `open_external` 命令，但 Web RichText 尚未接入；P5b 没有独立 origin、可撤销令牌及资源生命周期宿主。现有图片预览与 HTML 源码读取不是可执行 HTML 预览，不以 iframe/srcdoc 替代。P5b 在不改后端/宿主的本轮范围内阻塞。
- PI 六条实机旅程继续未验证，安装版、真实输入法、跨平台及真实服务门禁不因 mock 浏览器通过而关闭。
- 按最终收敛要求再次运行审批单例与 P2 面板测试，共 9 条通过，22.5 秒，退出码 0。本轮不提交混合工作树。
- 独立只读 code-reviewer 确认文本双呈现来自工具结果与助手输出；同时指出三个尚未修复的源码风险：去除乐观 `approval_decision` 后 reducer 可能保留终态工具的 pendingApproval 并被快照重写状态；ProductApp 的 mount-only media-query listener 可能捕获初始会话的面板 setter；移动端审批触发按钮消失后缺少焦点返回 fallback。这些未由本轮修复，不将 P2 整体标记完成。
- test-runner 全量验证：`pnpm test` 44 文件/339 条通过，退出码 0；`pnpm typecheck` 退出码 0；`pnpm test:e2e` 75 通过、1 失败、5 跳过，退出码 1。失败为 `workbench-composer.spec.ts:164` 的 busy 时输入框禁用断言；当前代码允许 busy 输入，停止后的断言尚未执行。早期专项通过不能替代本次全量结果，该用例/合同差异留待后续定位。本轮未运行 build、Rust 或真实服务门禁。
- busy 停止全量失败已定位并修复：原断言在运行中要求输入框禁用，与发送后短暂禁用态竞态；运行中允许继续输入是既有 Send Message 合同，停止按钮独立可用。用例改为等待输入恢复、断言停止按钮可用并点击、停止后输入可用且草稿保留；`--repeat-each=5` 5 次并行通过（18.6 秒，退出码 0）。test-runner 复验：`pnpm test:e2e` 76 通过、5 跳过、0 失败（1.0 分钟，端口 13187），`pnpm typecheck` 退出码 0。停止按钮仍为独立 `type=button`、busy 时可见、取消行为未削弱；仅测试合同更新，未改产品代码。

## 13. P3/P4 呈现层竞态守卫与调查结论

- P3–P5 源码调查（explorer f17c3c9d）修正了一处证据：Runtime SQLite 已持久化审批记录（`pending_approvals` 表），缺的是产品 API 的历史投影与权威决策事件，不是完全没有持久记录；完整决策 ID、actor、决策时间仍缺，不能从工具完成倒推授权。
- 按 plan §3“结果不匹配当前选择就不渲染”，为 FilesPanel/DiffPanel/ArtifactPanel 落地请求代（request-generation）守卫：工作区/目录/焦点/加载更多/打开/下载/会话刷新都开启新代，晚到响应不发布、加载更多不重复追加、作废的图片预览对象 URL 立即 revoke。修复前 `openFile`/`loadMore` 与 Diff/Artifact 面板的 `load` 均无身份校验。
- 验证：`tsc --noEmit` 退出码 0；vitest 44 文件/339 条通过，退出码 0。单测为静态渲染、无 jsdom，无法构造异步竞态用例，故不加竞态单测；改用真实行为断言的 e2e 不在 mock 能力内（mock 无文件/证据路由），扩大 mock 属超范围，如实记录该缺口。
- 边界：只做呈现层守卫，不改 API/schema/依赖；P3 的信任链路缺口、P4 决策历史投影缺失、P5b 无隔离预览宿主仍为开放项；现有 image preview 的 MIME 白名单与 CSP sandbox 维持原样。

## 14. P2 三个审查风险已修复（同轮追加）

第 12 节记录的三条未修风险全部关闭，均为呈现层，无 API/schema/Runtime 变更：

- **终态工具被快照改写**：`lib/rove-state.ts` 的 `upsertTool` 会把 `tool_call_approval_needed` 留下的 `pendingApproval` 保留到 `tool_call_completed` 之后的终态工具上，随后 `syncPendingApprovals` 在服务端快照不再列出该审批时把它改写回 `running` 并把详情换成通用 `Approval state synced`。新增 `isTerminalToolStatus` 守卫：终态（`done`/`error`）只丢弃过期 `pendingApproval`，保留原有结局与详情。回归测试先以移除守卫实测失败（状态被改为 `running`），再以守卫实测通过，因此不是空断言。
- **media query 闭包过期**：`shell/ProductApp.tsx` 的 960px 监听器只注册一次，闭包里是最首帧 `panel` 对象，缩放时操作的是另一个会话缓存的面板选择。改为经 ref 读取当前 panel。
- **移动端焦点回退缺失**：`inspector/use-work-panel.ts` 的 `close()` 原先只在记录到的触发元素仍连接时还焦；审批卡结算后其详情按钮会被卸载，焦点落到 body。`close()` 现按序回退到登记的稳定 shell 控件（`inspectorButtonRef`）。

验证（本 worktree，真实退出码）：`pnpm typecheck` 0；`pnpm exec vitest run` 44 文件/340 用例通过（新增 1 条）；`pnpm build` 0。

同轮还发现并修复一处先前会话遗留的编译阻断：`inspector/ArtifactPanel.tsx` 重复了一行 `if (!stale()) {` 且 `requestRef` 声明缺失，导致 `tsc` 失败、任何 Web 构建都无法进行。该文件是 P3 竞态守卫改动的落点，因此这个阻断使第 13 节声称的"`tsc --noEmit` 退出码 0"在本轮开始时并不成立。现已恢复为可编译状态。

## 15. P3 信任边界已闭合

第 13 节遗留的"P3 信任链路缺口"已定位并处置。子代理调查报告确认：`apps/api/src/product/files.rs` 的四个处理器从未调用 `state.project_trust()`、`resolve_product_workspace_trust` 或 `project_capability_allowed`，只靠全局 bearer/CORS 中间件与文件内的路径监禁、密钥名过滤和字节上限保护；MCP 与 run 创建路径则会查询信任。报告同时指出 ProjectTrust 的能力集（project_configuration、workspace_instructions、mcp_processes、hooks_extensions、provider_credentials、external_paths）在设计上不含通用文件读，因此这是边界问题而非 files.rs 的缺陷。

选择与 run 创建完全一致的判定：**Revoked 拒绝，Unknown/Restricted 保持可读**。

- Revoked → 409 `project_trust_required`，与 `lib.rs` 创建 job 前的拒绝使用同一 code 与文案；`decide_project_trust` 在 revoke 时本就会隔离该工作区的任务，此前只有文件浏览还开着。
- Unknown/Restricted → 保持可读。工作区根是用户经产品界面显式登记的，读这个根内文件属于 exact-root 信任；且 ProjectTrust 本身就是 capability-specific，Restricted 意为"根可信、能力未授予"。
- 未配置信任权威时**不** fail-closed。run 创建用 `state.project_trust()?` 会传播 503，但文件浏览若照做会让所有未注入信任仓库的嵌入方（以及默认 `ApiState::new`）对每次读取返回 503，这是移除一个可用边界而不是新增一个，因此只在权威存在且结论为 Revoked 时拒绝。
- 只查激活状态。`capability_digests` 不影响 `state`（见 `resolve_project_trust_record`），传空 map 即可避开 run 创建所需的按会话 provider selector，避免每次目录展开都要读取全部会话模型配置。

实现为 `apps/api/src/product/trust.rs::ensure_workspace_read_allowed`，由 listing 与 `resolve_workspace_file`（覆盖 content/download/preview）各调用一次；四处 utoipa 声明同步补 409。

验证：`cargo fmt --all --check` 0；`cargo clippy -p rove-api -p rove-integration-tests --all-targets` 干净；`cargo test -p rove-integration-tests --test api` 120 通过（新增 1 条）。负向用例断言四个面在 revoke 后均为 409 且带 `project_trust_required`，并断言 unknown 与 restricted 下 listing 为 200；移除守卫后实测该用例失败，因此是真实守卫而非摆设。

仍未做：ProjectTrust 从未覆盖"通用文件读"这一设计缺口本身；若未来决定把文件浏览纳入能力集，需要 schema 默认值、迁移与旧客户端兼容审查，不在本轮范围。

## 16. P4 授权历史：只呈现持久存在的请求侧

第 12、13 节遗留的"P4 决策历史投影缺失"已按计划允许的路径落地：查询端点未落地前，分区显示"历史不可用"，不猜测。

子代理调查报告给出的持久事实：`pending_approvals` 表有 `call_id/job_id/run_id/name/args_json/reason/status/created_at/updated_at`，决策只写进可变 `status` 字符串（approved/rejected/cancelled/interrupted）；`tool_call_approval_needed` 是唯一的审批相关 canonical 事件，没有 decision 事件；`POST /jobs/{id}/approvals/{call_id}` 的 body 只有 `decision`，无 actor；产品域没有任何审批历史端点。会话归属可以经 `product_session_runs` / `product_runtime_job_owners` 关联，但 runtime StateIndex 与 ProductStore 是两个独立 SQLite 库。

结论：请求侧已持久且已随产品 transcript 投影到每个产品会话，决策侧（决策、决策者、决策时间、结果归因）没有持久字段。因此新增 `apps/web/inspector/SessionAuthorizationPanel.tsx`：

- 读取产品 transcript，收集 `tool_call_approval_needed` 事件，按 call_id 去重、按 seq 倒序、上限 50 条；
- 展示工具名、调用 ID、run 序号与作用域、继承的 run 标记，以及服务端给出的 reason；
- 决策、决策者、决策时间三列固定渲染本地化"未知"，并在分区顶部用常驻说明写清"尚未持久化；工具执行成功不代表本窗口批准过它"；
- transcript 为 partial 时显式提示记录可能不完整；会话切换后不渲染旧会话记录。

明确不做：不从 `tool_call_completed` 反推授权；不新增决策事件、actor 字段、历史查询端点或 schema 迁移。这些是计划中独立的合同扩展阶段，需要单独的默认值/迁移/旧客户端兼容审查，未获确认前不动 Runtime 事件流。

验证：`pnpm typecheck` 0；`pnpm exec vitest run` 45 文件/345 用例通过（新增 5 条）；`pnpm build` 0；`pnpm test:e2e` 77 通过、5 跳过、0 失败。copy 双语字典键集一致性测试通过。

## 17. 本轮累计与仍未关闭项

本轮已完成并提交（分支 `feature/pi-desktop-workbench-p1a`）：

- `954e33a` 修复 ArtifactPanel 的编译阻断，使 `tsc` 与 Web 构建恢复可用；
- `1ca5c58` 关闭 P2 三个审查风险（终态工具快照改写、media query 闭包、移动端焦点回退）并补回归测试；
- `f684dcb` 闭合 P3 信任边界：revoked 工作区在四个文件读取面 fail-closed，并补负向用例与 OpenAPI 409；
- `ce7395e` 交付 P4 授权历史请求侧呈现，标注未知字段而非伪造。

仍未关闭（如实记录，不因本轮推进而改写）：

- P5a：安装版真实系统浏览器点击验收未跑。
- P5b：本批已落地隔离 origin 预览服务与 FilesPanel 入口，威胁模型门槛 1–6 见该文档 §8；安装版预览旅程与 macOS/Linux 仍未跑。
- P4 决策侧：本批已扩展 schema v5 `decided_via` 与 `GET /product/sessions/{id}/authorizations`；“决策者”仍是决策通道而非自然人。
- ProjectTrust 设计上不覆盖通用文件读这一缺口本身。
- PI-Desktop 六条实机旅程、安装版 Windows 旅程、真实输入法、macOS/Linux、外部 Provider 与真实第三方 MCP 门禁均未运行。

## 18. P5a 外链已接入受控宿主

第 17 节记录的 P5a 缺口已闭合。子代理调查确认：web 应用本身就是 Tauri 前端（`apps/desktop/tauri.conf.json` 的 `devUrl`/`frontendDist`），`open_external` 早已注册在 `apps/desktop/src/lib.rs:91`，Rust 侧用 `url::Url::parse` 强制 `http`/`https` 且必须有 host，`file:`/`javascript:`/`data:`/`mailto:` 一律拒绝；此前没有任何 Web 代码调用它，打包应用里点外链只会走 WebView 内导航。

改动全部在 web 侧，**无 Rust 变更、无新依赖、无命令注册表需要更新**：

- 新增 `desktopExternalLinkOpenerAvailable()` 与 `openDesktopExternalLink()`，沿用 `platform/desktop-commands.ts` 已有的 `desktopTransport()` 门与 `invoke` 注入模式。
- 返回值改为 `{status: "opened" | "blocked" | "failed" | "unsupported"}` 判别联合。Rust 返回 `Result<(), String>` 且只有两条不同错误文案，因此按文案映射 blocked 与 failed；这是为字符串匹配，脆弱性如实记录，若未来要稳定区分需把 Rust 侧改成类型化枚举——那属于跨面合同变更，不在本轮范围。
- `RichText.tsx` 的 `SafeLink` 仅在 `desktopExternalLinkOpenerAvailable()` 为真时 `preventDefault()` 并转交宿主；普通浏览器下保持原有 `target="_blank"` + `rel="noreferrer noopener"` 行为，不做任何宣称。
- 打开失败时经 copy 体系显示本地化原因，不静默吞掉。

安全边界未削弱：`safeRichTextUrl` 仍是唯一 scheme 闸门（只放行 `/`、`#`、`http(s)`、`mailto:`），`open_external` 又独立复校一次，因此 `javascript:`/`data:`/`file:` 即使绕过渲染层也到不了宿主。顺带修掉 `SafeLink` 把 react-markdown 的 MDAST `node` 透传成 DOM 属性的问题（原先会输出 `node="[object Object]"`）。

验证（本 worktree，真实退出码）：`pnpm typecheck` 0；`pnpm exec vitest run` 45 文件/350 用例通过（新增 5 条，含浏览器回退属性与危险 scheme 两条渲染断言）；`pnpm build` 0；`pnpm test:e2e` 77 通过、5 跳过、0 失败。

仍未做：UI 层未做真实系统浏览器点击验收（安装版 Windows 旅程未跑）；`opened` 不等于页面加载成功，这点已在代码注释与文案中写明。

（第 17 节"P5a 未接入"一条已由第 18 节闭合，此处不再作为开放项。）

## 19. Rust 全量门禁与三处既有失败

P3 是唯一改动 Rust 的阶段（`apps/api/src/product/{files,trust}.rs` + `tests/api.rs`），因此按计划 §5 跑了全量 Rust 门禁。

通过项（真实退出码）：

- `cargo fmt --all --check` 退出码 0；
- `cargo clippy --workspace --all-targets -- -D warnings` 干净，无 warning；
- `cargo test -p rove-integration-tests --test api` 120 通过（本次新增 1 条）；
- `cargo test -p rove-integration-tests --test tool_safety` 16 通过；
- `cargo test -p rove-integration-tests --test e2e` 113 通过；
- `cargo test -p rove-integration-tests --test mcp` 9 通过。

`cargo test --workspace --no-fail-fast` 报告 3 个失败，全部落在本次未触碰的 crate，且失败集合在两次运行间会变化（`rove-models` lib 一次 2 失败、一次 1 失败），属既有偶发：

- `provider::transport::tests::transport_injects_auth_and_drives_fragmented_sse`
- `provider::transport::tests::transport_redacts_and_bounds_error_body_before_protocol_classification`
- `tools::mcp::client_tests::an_unreachable_endpoint_is_reported_as_retryable_not_indeterminate`

判定依据（不是推测）：`git diff --stat 7e54ab6..HEAD -- models/` 与 `-- runtime/` 均为空，即这两个 crate 与基线逐字节相同，因此在 HEAD 上运行它们等于在 `main` 上运行；这三个用例全部是本机回环套接字/不可达端点驱动的时序敏感测试（失败断言位于 `models/src/provider/transport.rs:730`，等待一个分片 SSE 流产出 `TextDelta{text:"hi"}`）。本轮未在独立基线 worktree 复跑，但源码同一性已足以排除本次改动的影响。

不宣称：这 3 个用例在本机通过；也未在 macOS/Linux 运行任何 Rust 门禁。

## 20. P5b 威胁模型（未实现，未授权实现）

按计划 P5b"先写威胁模型"的要求，新增
`docs/design/2026-09-17-pi-desktop-workbench-p5b-threat-model.md`。结论：

- 唯一不可让步的约束是 **origin 隔离**：预览页脚本若运行在产品 origin 上，即可读取 `window.__ROVE_TOKEN__` 并以用户身份调用产品 API（攻击面 A1/A2）。这排除了用 `iframe`/`srcdoc` 内嵌当预览的做法。
- 落点选定 **`apps/api` 内新增受控路由**，理由是 `files.rs` 的 `join_safe`、`is_secret_filename`、字节上限与 `serve_file` 可原样复用，A3/A4/A6/A10 的缓解因此是复用而非重写；独立进程反而多一条进程间通道与一套独立路径校验。
- 已记录 13 项攻击面及各自缓解，其中 A5（SSRF/数据外传）以"不代取远程内容 + CSP `connect-src 'none'`"排除，A9（自动运行 package scripts）以"只服务已有静态文件、不触发构建"排除。
- 明确不做：通用远程代理、自动构建、以 iframe 冒充可执行预览。
- 列出 6 条实施门槛（origin 与预览令牌且有测试证明产品令牌在预览 origin 不可用、复用 `join_safe` 的负向用例、令牌生命周期与撤销、资源上限、未信任工作区拒绝、`docs/runtime/` 与 OpenAPI 同步）。

**P5b 仍未实现，本模型不授权实现。** 门槛 1 依赖浏览器同源策略按预期执行，需真实浏览器用例证明，不能只靠代码审读。

## 21. P6 综合验收：脚本在本机无法产出判定

按计划 P6 运行了 `scripts/product-acceptance.ps1 -SkipBrowser`。**判定为 FAIL，但这个 FAIL 不是代码失败，而是脚本在本机无法读取退出码。**

现象：12 个检查项全部报 `error` 且 `exit_code` 为空。逐项查看 `.rove/acceptance-logs/*.out.log` 与 `*.err.log`，实际结果全部是通过的：

- `fmt` 无输出（即无 diff）；
- `clippy` `Finished dev profile`，无 warning；
- `test-mcp` `9 passed`；`test-api` `120 passed`；`test-e2e` `113 passed`；`test-tool-safety` `16 passed`；`test-product-store` `130 passed`；
- `web-typecheck` 无错误输出；`web-test` `46 files / 354 tests passed`；`web-build` 正常产出路由清单。

根因（已隔离复现）：脚本用 `Start-Process -PassThru` 取 `$process.ExitCode`，而本机是 Windows PowerShell **5.1.26100.9444 (Desktop)**。最小复现：以 `Start-Process` 启动 `cmd.exe /c exit 0`，`WaitForExit()` 与 `Refresh()` 之后 `ExitCode` 仍为空、`HasExited=True`。这是 PS 5.1 的已知行为，因此脚本里"退出码不可判定即记为 error、绝不默认通过"的分支被普遍触发。

处理：按仓库规则**没有手改 `PRODUCT_ACCEPTANCE_REPORT.json`**，也没有把脚本的 FAIL 当作真实失败或真实通过。脚本本身的修复超出本轮范围（它是跨阶段共享工具，且 CI 环境可能不受此问题影响），此处仅记录现象与根因。

因此 P6 的门禁证据以**本会话直接运行各检查项取得的真实退出码**为准，见第 19 节与下表：

| 检查 | 真实结果 |
|---|---|
| `cargo fmt --all --check` | 退出码 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 干净，无 warning |
| `cargo test -p rove-integration-tests --test api` | 120 通过 |
| `cargo test -p rove-integration-tests --test mcp` | 9 通过 |
| `cargo test -p rove-integration-tests --test e2e` | 113 通过 |
| `cargo test -p rove-integration-tests --test tool_safety` | 16 通过 |
| `cargo test -p rove-api --lib product:: -- --test-threads=1` | 130 通过 |
| `pnpm typecheck` | 退出码 0 |
| `pnpm test` | 46 文件 / 354 用例通过 |
| `pnpm build` | 成功 |
| `pnpm test:e2e` | 80 通过、5 跳过、0 失败 |

P6 仍未完成的部分：`-IncludeGated` 的 `mcp-filesystem-smoke`（真实第三方 MCP）未跑；安装版 Windows Desktop 完整旅程未跑；`web-e2e` 虽在单独运行时通过，但未纳入本轮脚本报告。`cargo test --workspace` 的第 19 节三处既有偶发失败依旧存在。

## 22. P4 决策侧合同与 P5b 预览服务收口（2026-09-22）

本批把第 17 节开放的 P4 决策侧与 P5b 实现收口，全部在 `feature/pi-desktop-workbench-p1a` 工作树内完成，**尚未提交**。

### P4 决策侧

- StateIndex schema v4→v5：`pending_approvals.decided_via`（可空；迁移前行保持 NULL，读者必须显示“未知”）。
- `record_approval_decision(call_id, status, decided_via)` 取代无决策通道的 `mark_pending_approval_status` 调用点：`job_api` / `job_cancel` / `job_responder_lost`。
- 新端点 `GET /product/sessions/{session_id}/authorizations`（默认 50、上限 200）：请求+决策投影，并按 `call_id` 关联 terminal `tool_call_completed`/`tool_call_failed` 作结果归因；未记录字段保持 `null`，不猜测。
- Web：`SessionAuthorizationPanel` 改读该端点，展示决策状态/通道/时间/结果；copy 双语更新。
- 证据：runtime `approval_*`/`tool_outcome_*` 4 例；`product_session_authorizations_*` 2 例；vitest 354 通过；`tsc --noEmit` 干净。

### P5b 预览服务

- `apps/api/src/product/preview.rs`：独立回环 origin（临时端口）、160-bit 会话令牌、TTL 30min、关闭即撤销、复用 `join_safe`/`is_secret_filename`/字节上限、CSP `connect-src 'none'`、无 BearerAuth/无 Set-Cookie。
- FilesPanel：HTML 文件“在隔离预览中打开/关闭”入口；面板卸载即撤销令牌。
- 证据：`product_preview_*` 6 例（含 traversal/secret/session-cap/CORS/409 trust/503 无 listener）；真实浏览器 gate1 `apps/web/tests/e2e/workbench-preview-origin.spec.ts` 通过（产品令牌在预览 origin 不可用、opener 读不到、产品 API 非 200）。
- OpenAPI 已注册 previews/authorizations 路径并通过契约测试。

### 本批验证汇总（2026-09-22）

- `scripts/product-acceptance.ps1`：**PASS（11 passed, 0 failed, 1 not run）**。`mcp-filesystem-smoke` 为 gated 检查未跑。ExitCode 工具修复已验证（本轮报告含真实退出码）。
- `pnpm test:e2e`：通过（含 `workbench-preview-origin.spec.ts` gate1）。
- `cargo test --workspace --no-fail-fast`：全部 `0 failed`。
- `cargo fmt --all --check` / `cargo clippy --workspace --all-targets -- -D warnings`：干净。

### 本批未跑 / 未关

- `mcp-filesystem-smoke`（真实第三方 MCP，`-IncludeGated`）。
- 安装版 Windows Desktop 完整旅程、macOS/Linux、外部 Provider。
