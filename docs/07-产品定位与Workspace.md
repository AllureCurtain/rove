# rove — 产品定位与 Workspace 抽象

> 这份文档回答一个上层问题:**rove 到底是什么产品**。它约束后续执行内核、工具、记忆、接口层的设计,避免项目自然滑成"又一个 Claude Code"或"泛聊天壳"。

---

## 1. 一句话定位

> **rove 是一个本地优先、可恢复、可观测的通用 agent runtime,通过 Workspace adapter 进入不同工作世界。**

第一阶段先做 `Folder/Repo Workspace`,所以它会像 Claude Code 一样在本地目录里工作。但 rove 的核心定位不是 code-only agent,而是一个可以挂载不同工作世界的 agent runtime。

---

## 2. 为什么需要这层定位

`06-请求生命周期.md` 已经把一次请求从启动到退出的 12 站设计清楚了。这套设计工程上成立,但它主要回答的是:

- 请求怎么进来
- Engine 怎么跑
- 工具怎么调用
- 状态怎么落盘
- 事件怎么渲染
- 进程怎么退出

它没有充分回答:

- agent 面对的"世界"是什么
- 代码仓库是不是唯一世界
- 普通文件夹、任务沙箱、桌面、浏览器怎么接入
- rove 和 Claude Code 的边界在哪里

`Workspace` 就是补上这层产品抽象。

---

## 3. 核心抽象:`Workspace`

`Workspace` 表示 agent 当前工作的世界边界。

它负责回答三个问题:

1. **在哪里工作**:根路径、会话目录、状态目录在哪里
2. **面对什么世界**:普通文件夹、repo、任务沙箱、桌面、浏览器
3. **具备哪些能力**:文件读写、git、shell、浏览器、桌面自动化等

核心类型应该叫 `Workspace`,不要叫 `RepoWorkspace`。

理由:

| 命名 | 问题 |
|---|---|
| `RepoWorkspace` | 把产品心智提前锁进代码仓库,未来接普通文件夹 / 桌面 / 浏览器会别扭 |
| `FolderWorkspace` | 适合描述第一版实现,但不适合做核心概念名 |
| `Workspace` | 足够通用,能容纳不同工作世界 |

---

## 4. WorkspaceKind

第一版只需要两个 kind:

| Kind | 含义 | M0-M1 行为 |
|---|---|---|
| `Folder` | 普通本地文件夹 | 文件读写、搜索、状态落盘 |
| `Repo` | 带 git 语义的文件夹增强模式 | 在 Folder 基础上增加 git 探测、项目记忆、测试命令等 |

后续可扩展:

| Kind | 含义 | 何时考虑 |
|---|---|---|
| `Task` | rove 为一次任务创建的工作沙箱 | 当用户不想先进入某个目录,只想丢一个任务 |
| `Desktop` | 本机桌面 / 应用窗口环境 | 做 Codex Desktop 类自动化时 |
| `Browser` | 浏览器上下文 | 做网页任务、资料收集、Web 自动化时 |

关键点:`Repo` 不是 `Workspace` 的同义词,只是 `Folder` 的增强形态。

---

## 5. 两种入口

### A. 从目录启动

用户在一个本地目录里运行:

```bash
rove "帮我理解这个项目"
```

rove 会:

1. 识别当前目录为 workspace root
2. 向上探测 `.git` / `.rove`
3. 如果发现 git,标记为 `WorkspaceKind::Repo`
4. 在 `.rove/` 下写入 run state、trace、report、memory

这是 M0-M1 的默认路径。

### B. 从任务启动

用户直接给一个任务:

```bash
rove run "整理这些资料并输出一份报告"
```

rove 可以为这次任务创建一个 task workspace。

这条路径不要求 M0 实现,但核心概念必须容纳它。否则 rove 会被 CLI repo agent 的早期形态锁死。

---

## 6. 与 Claude Code 的关系

rove 第一阶段可以像 Claude Code,但不应该被定义为 Claude Code clone。

| 维度 | Claude Code-like 部分 | rove 的上层边界 |
|---|---|---|
| 默认工作方式 | 在目录 / repo 内工作 | 这是第一种 workspace,不是全部 |
| 工具能力 | 文件、shell、git、搜索 | 未来可扩展桌面、浏览器、任务沙箱 |
| 内核 | 流式事件、工具调用、状态 | 保持 runtime 化,接口只是壳 |
| 记忆 | 项目上下文 | 应该属于 workspace,不是只属于 repo |
| 产品定义 | coding agent | workspace agent runtime |

所以 M0-M2 做得像 Claude Code 是合理的,但类型命名和文档心智不能把 rove 锁死成 code-only。

---

## 7. 对现有架构的约束

后续 `01/04/06` 中的实现设计必须遵守:

1. 核心概念是 `Workspace`,不是 `RepoWorkspace`
2. `Repo` 是 `WorkspaceKind` 或能力增强,不是唯一工作世界
3. `.rove/` 状态目录属于 workspace
4. memory / trace / report 绑定 workspace + run,不只绑定代码仓库
5. 工具注册可以根据 workspace kind / capability 裁剪
6. CLI、API、Web 都消费同一个 runtime,只是创建 workspace 的方式不同

---

## 8. 非目标

- 不在这份文档里展开完整请求生命周期
- 不在 M0 做 desktop / browser 自动化
- 不把 rove 定位成纯研究型 agent 实验台
- 不把 rove 定位成只会代码的 agent
- 不在第一阶段做 multi-agent

---

## 9. 当前锁定决策

| # | 决策 | 选择 |
|---|---|---|
| 7.1 | 产品定位 | 本地优先的通用 agent runtime |
| 7.2 | 核心抽象名 | `Workspace` |
| 7.3 | 第一阶段 workspace | `Folder/Repo Workspace` |
| 7.4 | Repo 语义 | `Folder` 的增强形态,不是核心同义词 |
| 7.5 | 后续工作世界 | `Task` / `Desktop` / `Browser` |
| 7.6 | 与 Claude Code 关系 | 第一阶段可像,但不定义为 clone |
| 7.7 | 12 站设计处理 | 保留,作为 runtime 细节,受本定位约束 |

---

## changelog

- 2026-05-18:初版,确认 rove 的上层定位为 Workspace Runtime。
