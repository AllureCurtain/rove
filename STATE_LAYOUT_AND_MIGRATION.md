# Rove 状态目录布局与迁移(State Layout and Migration)

> Status: **Implemented**(实现于 `feature/user-state-migration`,基线 `5fe9d70`)
>
> 设计记录: [`docs/design/2026-08-16-user-state-directory-migration-design.md`](docs/design/2026-08-16-user-state-directory-migration-design.md)
> 任务计划: [`docs/plans/2026-08-16-user-state-directory-migration.md`](docs/plans/2026-08-16-user-state-directory-migration.md)

本文描述当前实现的用户级运行数据目录合同与旧 `.rove/` 迁移行为。代码
事实来源是 `apps/bootstrap/src/user_state.rs`、`apps/bootstrap/src/state_migration.rs`
与 `apps/bootstrap/src/config.rs`;本文与 `docs/runtime/` 同步维护。

## 1. 目录归属总览

| 数据 | 位置 | 说明 |
|---|---|---|
| Provider catalog / 用户配置 | `~/.rove/config.toml`(`ROVE_CONFIG_ROOT` 覆盖) | 未变;credential 只存引用 |
| Project Trust 库 | Windows `%LOCALAPPDATA%\rove\project-trust.sqlite`,其他平台 `$XDG_STATE_HOME/rove/`(`ROVE_PROJECT_TRUST_STORE` 覆盖) | 未变 |
| **运行状态**(默认) | `<data_root>/workspaces/<storage_key>/` | 本任务引入;见 §2 |
| **ProductStore** | `<data_root>/product.sqlite` | API-global;不按 workspace 复制 |
| 项目共享配置 | 项目内 `.rove/config.toml`、`.env`、`AGENTS.md`、AgentDefinition 包 | 留在项目内,全部受 Project Trust 能力门约束 |
| 显式项目 MCP catalog | 仅当 `tool.mcp_config_path` 显式指向项目路径时留在项目内 | 继续受 `mcp_processes` Trust 能力门约束；未显式配置的 legacy `.rove/mcp_servers.json` 是迁移源 |
| Desktop 自有目录 | `%APPDATA%\Rove\{config,state,logs}`(Windows)/ 平台等价物 | Desktop 令牌与日志;其 state 覆盖保持显式注入语义 |
| benchmark 产物 | 显式 evidence 目录(如 `benchmarks/results/`) | 一次性产物,不进入用户数据目录 |

`data_root` 平台默认:Windows `%LOCALAPPDATA%\rove`(回退 `%APPDATA%\rove`);
macOS `~/Library/Application Support/rove`;Linux `${XDG_DATA_HOME:-~/.local/share}/rove`。
覆盖:`ROVE_DATA_ROOT` 环境变量或 `AppConfigOverrides.data_root` 显式注入,必须绝对路径,
否则 fail-closed。生产 `AppConfig::load` 在无法解析用户根时拒绝启动；仅
`AppConfig::default()` 等显式 programmatic/embedding 构造保留旧 `.rove` 路径，
以维持无用户环境的确定性测试与兼容调用。用户 data root 不能位于所选
workspace 内，避免迁移目标被再次纳入 legacy 源清单。

## 2. 契约布局(user contract v1)

```text
<data_root>/
  product.sqlite                  # API-global ProductStore(原 legacy product.sqlite)
  workspaces/<storage_key>/
    workspace.json                # 身份标记(schema 1:canonical_root + storage_key)
    state.sqlite                  # 运行索引(原 .rove/state.sqlite)
    mcp_servers.json              # MCP catalog(原 .rove/mcp_servers.json)
    runs/<run_id>/…               # trace.jsonl / task_state.json / report.json / tool_artifacts/(内部布局不变)
    memory/MEMORY.md、memory/topics/、memory/sessions/
    session-model-selections/
    circuit_breakers.json
    tasks/<name>/…                # Task 工作区 base(默认)
    repl_history
    .migration/                   # 迁移 journal、回执、锁、冲突备份
```

- `storage_key = hex(stable_hash(canonical_root_key + "|" + kind))[..16]`;
  kind 由 `.git` 存在性决定(`repo`/`folder`),输入经 `canonicalize()` 归并
  symlink/reparse point。同一 workspace 的任何入口(cwd、API 绝对根、Desktop)
  得到同一目录;不同 canonical 路径不碰撞,`workspace.json` 标记做碰撞兜底。
- 该 key 与 Project Trust 的 identity digest 是**不同用途的摘要**:trust digest
  额外绑定文件系统身份(dev/ino 或 creation time)用于检测目录被替换。
- workspace 移动/重命名 → 新 key → 视为新 workspace;旧数据留在原处,
  由迁移命令按 legacy 发现处理(显式边界,不做 move-following)。

### 配置语义

`state.state_dir`、`state.sqlite_path`、`memory.session_dir`、
`memory.durable_dir`、`tool.mcp_config_path` 的默认值是**空哨兵** = 走契约。
显式配置的值继续按现行语义解析(相对 → workspace 根;绝对 → 需
`state.allow_external_paths` 或用户配置已加载),`rove state paths` 会把显式
项目内布局标为 legacy 诊断。`AppConfig` 的访问器
(`state_dir()`、`sqlite_path()`、`memory_paths()`、`mcp_config_path()`、
`product_sqlite_path()`)实时解析,`rebase_to_workspace` 后自动跟随新根。

### MCP catalog

- 默认 catalog 位置迁到契约目录;`mcp_processes` 能力门不变:启动 MCP 进程
  仍需逐 workspace 授权。
- 读取顺序(未显式配置时):契约文件存在则读契约;否则读 legacy
  `.rove/mcp_servers.json`(升级后、迁移前的连续性);都没有则无 MCP。
  契约文件一旦落盘(首次 Settings 写入或迁移),legacy 永远不再被读取,
  Settings 删除服务器不会复活旧 catalog。
- 不存在 catalog 的 list/read 是零副作用操作，不会为了返回空列表而创建
  contract 父目录。Product Settings 首次 mutation 先校验请求，再创建/校验
  `workspace.json` marker，在目标锁内把当前生效的 legacy catalog 提升一次，
  随后对 contract 文件执行 mutation；已经存在的 contract 永远不被后续
  legacy 内容覆盖。mutation 后同时清除 legacy 与 contract 两个 health-cache
  key，避免路径切换后沿用旧诊断。
- Trust digest 对齐:`digest_mcp_configuration` 摘要"配置指针标签 + 生效
  catalog 内容"。逐字节迁移后既有 `mcp_processes` 授权 digest 不变——
  迁移既不自动授予也不静默失效;显式项目 catalog 的语义不变。
- 契约 catalog 在 workspace 之外,由 bootstrap 侧有界读取(256 KiB 上限)
  后交给 runtime 的 servers 级注册 API;Execution Environment 的
  workspace 相对读边界保持不变。

## 3. 迁移(`rove state migrate`)

```text
rove state paths                 # 解析结果 + workspace 身份(pretty JSON)
rove state migrate               # dry-run(默认):零写入
rove state migrate --apply       # 执行
rove state migrate --apply --on-conflict backup-target
rove state migrate --apply --prune-legacy
```

- **分类**:state_sqlite、product_sqlite、mcp_catalog、memory、run_artifact、
  selection_store、health_store、task_workspace(不透明整树)、repl_history、
  unknown(原样复制并显式列出)。`config.toml` 与 sqlite 的 `-wal/-shm`
  影子文件不迁移;memory 替换事务的 `.tmp/.bak/.ready` 残留计为 risk。
- **幂等基准是逐文件 sha256**,不是事件计数:目标存在且同 hash → skip;
  不同 → conflict;不存在 → 复制(原子 tmp+rename,unix 0600)。
- **SQLite**:始终使用 `VACUUM INTO` 建立只读一致快照，避免主文件与活跃
  WAL 分离。`state.sqlite` 中历史绝对 `run_dir`、trace/task/report 索引路径
  会在临时快照内事务化重定位到新 state 根，再经 `PRAGMA quick_check`、
  文件同步和原子 rename 落盘；源根以外的路径保持不动。journal 先同步一条
  `prepared` source/target digest，关闭 rename 后、最终 outcome 前退出的恢复
  窗口。目标打开时由现行 schema 迁移逻辑接管；高于当前版本的源库在计划
  阶段列为 risk，打开时按现行语义拒绝。
- **冲突**:默认 `keep_target`(目标不动、源可读、报告非干净、退出码非 0
  并带 `state_migration_conflict`);`backup-target` 把差异目标移入
  `.migration/conflicts/` 后复制源。任何策略都不静默覆盖。由于
  ProductStore 是全局库，第二个 workspace 携带 legacy `product.sqlite` 时
  会看到同一个全局目标冲突；不会静默合并或覆盖，使用 `backup-target` 前应
  先停止 API 并保留现有 ProductStore 备份。
- **并发**:`<workspace_dir>/.migration/lock` flock(5s 超时,超时为
  `state_migration_locked`)。当源包含 `product.sqlite` 时，另外持有
  `<data_root>/.migration/product.lock`，因为 ProductStore 是 API-global，
  不同 workspace 的迁移也必须串行。
- **journal/回执**:`.migration/journal.jsonl`(逐文件 prepared/结果记录，
  单行损坏不阻塞重跑；总大小硬限 64 MiB)、`.migration/migration.json`(完成回执)、源侧
  `.rove/.rove-migration-receipt.json`(全部成功后才写;"两边都有数据"时
  判定已迁移)。中断后重跑 `--apply`:hash 比较保证不重复、不覆盖。
- **复制语义 + 默认 `legacy_disposition: kept`**:失败时旧数据始终可读;
  回滚 = 忽略/删除目标。`--prune-legacy` 仅在无未解决冲突且逐文件复核
  (sqlite 快照类改用可打开校验)通过后删除已迁移文件,永不删除项目配置
  与 `.env`;与 `state repair`/`state cleanup` 是三个独立动作。
- dry-run 硬约束:不建目录、不写文件、不开写事务、不启动 Provider/MCP、
  不改 Trust、不调用模型。

## 4. 入口行为

所有生产入口(CLI exec/REPL/TUI/sessions/state/trust、API
serve/embedded、per-job rebind、M1 import、bench)通过
`AppConfig` 访问器获得同一解析结果;契约生效时入口额外执行
`ensure_workspace_layout`(建目录、unix 0700、写身份标记)。
不自动迁移:检测到 legacy `.rove/` state 时新写入仍走契约目录,旧数据
保持可读,`rove state paths` 显示 legacy 状态并提示迁移;迁移前旧 run
不可 resume(resume 只查活跃 state 库,fail-closed 语义),执行
`rove state migrate --apply` 后即可精确 resume(工件逐字节复制,
`RuntimeIdentity` 输入不变)。checkpoint 内嵌的 `.rove/memory/...` 指针
字符串是记录性内容,迁移不改写,运行时一律经 `MemoryPaths` 绝对路径解析。

## 5. 测试与证据

- 契约/迁移行为测试:`apps/bootstrap/tests/state_migration.rs`(23 例:
  fresh、dry-run 零副作用、分类、apply+幂等/续号、普通文件与 SQLite prepared
  中断恢复、损坏 journal/SQLite、并发锁、无效 data root、冲突 keep/backup、
  SQLite 替换冲突、全局 ProductStore 冲突、一次性 prune、unknown partial prune、
  data-root/legacy/target/迁移元数据 symlink 边界、sqlite 可用性、精确 resume、
  MCP digest 存活)与
  `user_state.rs`/`config.rs` 单测(根解析 fail-closed、key 稳定/隔离/symlink、标记冲突、哨兵解析、
  显式覆盖兼容、校验跳过哨兵)。
- 入口接线断言:`apps/cli/src/cli/runtime.rs` 测试断言契约目录生效且不
  再创建 `.rove`。
- MCP 连续性断言:`runtime/src/tools/mcp_config.rs` 覆盖缺失 catalog
  零写读取和一次性 promotion；`tests/api.rs` 覆盖首次 Settings 写入保留
  legacy server、创建有效 marker 且不改写 legacy 源。
- Windows 可复跑脚本:`scripts/state-migration-smoke.ps1`(真实启动
  `rove state paths/migrate`,覆盖 fresh/dry-run/apply/幂等/冲突/prune 与
  迁移后路径检查)。精确 resume 由上述真实 `StateStore` Rust 回归测试覆盖，
  smoke 脚本不伪造 run 工件。
- Web 契约门禁通过 `pnpm test`(241 tests)、`pnpm typecheck`、`pnpm build`
  与 `pnpm test:e2e`(56 passed,5 个 real-API gate 在该 mocked run 中跳过)；
  随后的隔离 `local-full` 在非默认端口真实执行并通过全部 5 个 real-API
  Playwright 场景。两个 integration runner 都显式设置 disposable
  `ROVE_DATA_ROOT`，不接触操作者 ProductStore。
