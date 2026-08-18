# 用户级运行数据目录与旧 `.rove/` 迁移设计

> Status: **Implemented**
>
> Date: 2026-08-16
>
> Base: `feature/user-state-migration` at `5fe9d70`(基于 main `f6676d1`)
>
> Plan: [2026-08-16-user-state-directory-migration.md](../plans/2026-08-16-user-state-directory-migration.md)

本文档是迁移计划要求的目录归属设计记录。第 1 节保留了基线
`5fe9d70` 的历史审计；截至 2026-08-18，路径解析、契约布局、CLI 迁移、
marker、冲突/prepared journal、SQLite 索引路径重定位、MCP 生效路径和
首次 Settings 写入的 legacy catalog promotion、生产入口接线已经在本
worktree 实现。完整 Desktop 安装旅程、外部
Provider/MCP 互操作和 macOS/Linux 发布属于未运行的可选平台/外部 gate，
不影响本计划实现状态，也不把未运行 gate 写成互操作证据。

## 1. 当前路径现状审计(基线 `5fe9d70`)

当前并存四种目录约定:

1. **项目本地 `.rove/`**:CLI/API 独立运行和每个 rebind workspace 的默认
   state 根(`runtime/src/workspace/root.rs:44,60,77,96`;
   `apps/bootstrap/src/config.rs:440-441` 默认 `state_dir=".rove"`,
   `sqlite_path=".rove/state.sqlite"`,经 `resolve_path` 落在 workspace
   根下)。
2. **用户配置根 `~/.rove/`**:`ROVE_CONFIG_ROOT` 覆盖,否则
   `USERPROFILE|HOME` + `.rove`(`apps/bootstrap/src/user_config/paths.rs:14-77`),
   含 `config.toml`、`config.toml.lock`、`migrations/provider-*.json`。
3. **操作员 state 根**:`%LOCALAPPDATA%\rove\project-trust.sqlite` /
   `XDG_STATE_HOME|~/.local/state`(`apps/bootstrap/src/project_trust.rs:250-263,1233-1250`)。
4. **Desktop 自有约定**:`%APPDATA%\Rove\{config,state,logs}`
   (`apps/desktop/src/config.rs:38-123`),API embedded host 用
   `embedded_api_state(state_dir=%APPDATA%\Rove\state)`
   (`apps/api/src/lib.rs:374-402`)。

### 1.1 逐路径归属表

**A. 已在用户/操作员目录 —— 位置不变,不迁移**

| 路径 | Owner(代码) | Authority | 生命周期 | 敏感级 | 迁移策略 | 主要消费者 |
|---|---|---|---|---|---|---|
| `~/.rove/config.toml`(+`config.toml.lock`) | `UserConfigPaths`(bootstrap `user_config/paths.rs`) | 用户 | 持久 | 高(profile + credential reference) | 不迁移 | `ProviderCatalogService`、`AppConfig::load`、CLI/API 启动 |
| `~/.rove/migrations/provider-<digest>.json` | `provider_migration.rs:809-814` | 用户 | 持久回执 | 中 | 不迁移 | `rove provider migrate` |
| `<operator_state_base>/rove/project-trust.sqlite` | `ProjectTrustRepository::operator_default`(`project_trust.rs:250-263`) | 操作员 | 持久 | 高 | 不迁移(已用户级) | trust 命令、`AppConfig` load、API |
| `%APPDATA%\Rove\config\desktop.json`、`logs\` | desktop `config.rs` | 用户 | 持久 | 高(bearer token) | 保留 Desktop 自有;state 部分见 §4.8 | Desktop host |

**B. 项目来源 —— 留在项目内,继续受 Project Trust 约束**

| 路径 | Owner | Authority | 敏感级 | 迁移策略 | 依据 |
|---|---|---|---|---|---|
| `.rove/config.toml` | `AppConfig` project layer(`config.rs:671-675`) | 项目 + Trust `project_configuration` | 中 | 留在项目,不迁移 | `filtered_project_config` 已剥离 state/memory/allow_external 字段(`config.rs:1742-1754`),未信任时不可读 |
| `.env`(workspace 根) | `load_project_environment`(`config.rs:664-670`) | 项目 + Trust `provider_credentials` 等 | 高 | 留在项目 | 仅授权后进入 `ProjectEnvironment` |
| `AGENTS.md`、AgentDefinition 包、procedures | `agents/instructions` | 项目 + Trust `workspace_instructions` | 低 | 留在项目 | 现行能力门 |
| `.rove/` 目录名本身 | `environment.rs:2362` 敏感穿越名单、`executor.rs:164-166` 输出敏感过滤、`instructions.rs:637-642` 跳过名单 | — | — | 名单继续保留 `.rove` | 迁移后项目 `.rove` 仍是配置来源,过滤不应放松 |

MCP catalog 的归属修订依据:计划的迁移冲突场景清单明确列出"MCP 配置"
(plans/2026-08-16-user-state-directory-migration.md"迁移与恢复"一节),
只有当它存在迁移源与目标时该场景才成立;且现状中 catalog 由 Web Settings
按 workspace 管理(CDH "workspace-scoped Settings/MCP management",
`product/platform.rs` 经 `workspace_bounded_mcp_config_path` 写入),
本质是产品运行配置而非随项目共享的版本库内容。因此默认迁移到用户目录;
`mcp_processes` 能力门与启动约束原样保留,显式配置项目路径的团队仍可使
用项目内 catalog(向后兼容,与 `StateConfig` 显式覆盖同一规则)。

**C. 项目本地运行数据 —— 迁移到用户目录(本设计核心)**

| 路径 | Owner | Authority | 生命周期 | 敏感级 | 迁移策略 |
|---|---|---|---|---|---|
| `.rove/state.sqlite`(+wal/shm) | `StateIndex`(`runtime/src/state/index.rs:332-338`) | runtime state | 持久 | 中高(含 run/job 索引、对话消息) | 复制迁移(SQLite 快照,见 §5.4) |
| `.rove/runs/<id>/trace.jsonl`、`task_state.json`、`report.json` | `RunStore`/`TraceWriter`/`StateStore`(`state/trace.rs:24,82,95-97`、`state/store.rs:92-94`) | runtime state | 持久 | 中高(轨迹含工具输出) | 逐文件 hash 迁移 |
| `.rove/runs/<id>/tool_artifacts/**` | `ToolArtifactStore`(`state/tool_artifacts.rs:61-64,161-163`) | runtime state | 持久 | 中 | 逐文件 hash 迁移 |
| checkpoint | 内嵌于 `task_state.json`(`state/artifacts.rs:650-668`) | runtime state | 持久 | 中 | 随 run 迁移,内容不改写 |
| legacy `.rove/product.sqlite` | `SqliteProductStore`(`apps/api/src/lib.rs:484`) | ProductStore | 持久 | 高(会话/偏好/映射) | 复制到 API-global `<data_root>/product.sqlite`;不在 workspace 目录下复制 |
| `.rove/memory/MEMORY.md`、`topics/`、`sessions/` | `MemoryPaths`(`memory/paths.rs:13-20`;默认 `config.rs:429-434`) | memory | 持久 | 中 | 逐文件 hash 迁移 |
| `.rove/memory/.memory-index-*.tmp/.bak/.ready` | `memory/management.rs:22-25` | transient(崩溃恢复标记) | 进程 | — | 不迁移;残留计入迁移报告 risk |
| `.rove/session-model-selections/<sid>.json`(+lock) | `SessionSelectionStore`(`apps/bootstrap/src/session_selection.rs:44,88`) | bootstrap | 持久 | 低 | 复制迁移 |
| `.rove/circuit_breakers.json` | `ModelHealthStore`(`models/src/health.rs:40-45`) | models health | 持久可重建 | 低 | 复制迁移 |
| `.rove/tasks/<name>/**` | API task base(`apps/api/src/lib.rs:3708`;CLI `runtime.rs:305-315`) | workspace-owned(Task 工作区根) | 持久 | 中 | 整树递归复制(作为不透明目录,不下钻语义) |
| `.rove/mcp_servers.json` | `ToolConfig.mcp_config_path` 默认(`config.rs:410`) | MCP catalog | 持久 | 中(进程启动定义) | **迁移到用户目录**(§2);显式配置了项目路径的旧配置继续按现行语义工作 |
| `.rove/repl_history` | CLI repl(`apps/cli/src/cli/repl.rs:132`) | 用户 | 持久 | 低 | 复制迁移 |

**D. 进程内 / 一次性 —— 不迁移**

| 项 | Owner | 迁移策略 |
|---|---|---|
| approval / user-input 进程内状态 | Engine 内存 | 不迁移;不得伪装成 durable authority(现行语义不变) |
| 临时 trust(`ROVE_TRUSTED_WORKSPACES`、`--trust-project`) | `ProjectActivation::resolve`(`project_trust.rs:137-185`) | 不迁移;迁移不得授予任何 capability |
| benchmark evidence(`benchmarks/results/**`) | `apps/bench/src/runner.rs:122-135`(显式 `task_dir/.rove`) | 不迁移;bench 使用显式 state_dir,属"一次性 benchmark 产物" |
| evidence export | API 流式附件(`product/export.rs:275-283`) | 不迁移;不产生本地文件 |
| CLI/API 日志 | stderr,无日志文件(Desktop logs 除外) | 不变 |

### 1.2 硬编码 `.rove` 的全部触点(迁移后需逐一处理或确认保留)

- `runtime/src/workspace/root.rs:44,60,77,96` —— `Workspace` 四个构造器的
  state_dir 默认。
- `runtime/src/engine/facade.rs:208-213` —— `Engine::new` 的 cwd 回退
  (仅直接嵌入用;生产入口全部走 `with_workspace*`,见
  `apps/bench/src/runner.rs:516`、`apps/bootstrap/src/assembly.rs:102`、
  `apps/cli/src/tui/app.rs:2175,2790`)。
- `apps/bootstrap/src/config.rs:410,430-431,440-441,490,671-674,841` ——
  默认值与 project config 路径。
- `apps/api/src/lib.rs:108,396,475,3525`、`product/migration.rs:294,489` ——
  API 启动、embedded host、M1 校验、错误文案。
- `apps/cli/src/cli/provider.rs:34`、`args.rs:123` —— legacy product store
  默认来源(迁移命令的 legacy 发现点)。
- `apps/bootstrap/src/project_trust.rs:1032-1039`(digest 默认路径)、
  `apps/bootstrap/src/registry.rs:101-120`(MCP 装配)—— MCP catalog
  迁移后需随生效路径解析。
- `runtime/src/memory/durable.rs:114`、`session.rs:9,39`、
  `state/report.rs:22`、`state/trace.rs:73`、`state/artifacts.rs:790-791` ——
  文档注释与 checkpoint 内嵌指针字符串 `.rove/memory/...`(内容不改写,见 §6.5)。
- `apps/bench/src/runner.rs:126` —— 保留(显式本地布局)。

## 2. 当前实现布局（Implemented）

```
<data_root>/
├── product.sqlite                 # API-global ProductStore
└── workspaces/<storage_key>/
    ├── workspace.json             # 身份标记(见 §3)
    ├── state.sqlite               # 原 .rove/state.sqlite
    ├── mcp_servers.json           # 原 .rove/mcp_servers.json(MCP catalog)
    ├── runs/<run_id>/…            # 内部布局不变
    ├── memory/{MEMORY.md, topics/, sessions/}
    ├── session-model-selections/
    ├── circuit_breakers.json
    ├── tasks/<name>/…             # Task 工作区 base
    ├── repl_history
    └── .migration/{lock, journal.jsonl, migration.json}
```

`product.sqlite` 是全局目标；来自第二个 legacy workspace 的同名数据库不会
自动 merge，默认会报告冲突，只有显式 `backup-target` 才会替换并保留备份。

`data_root` 平台约定(与现有 `operator_state_base()` 的手写 env 解析风格
一致,不引入 `dirs` 依赖):

| 平台 | data_root | 依据 |
|---|---|---|
| Windows | `%LOCALAPPDATA%\rove`(缺失回退 `%APPDATA%\rove`) | 机器本地数据用 LOCALAPPDATA,与 trust store 的 base 选择一致(`project_trust.rs:1233-1242`) |
| macOS | `~/Library/Application Support/rove` | 平台数据约定 |
| Linux | `${XDG_DATA_HOME:-$HOME/.local/share}/rove` | XDG 数据约定;`XDG_STATE_HOME` 继续保留给 trust store(现状) |

- 覆盖:`ROVE_DATA_ROOT`,必须绝对路径,相对或无法解析时 fail-closed
  (复用 `UserConfigPaths::discover_from` 的失败语义,`user_config/paths.rs:55-86`)。
- 契约同时暴露语义子根 `config_root`(不变,`~/.rove`)、`data_root`、
  `cache_root`(`<data_root>/cache`,本期无使用者,不创建)、
  workspace-owned(项目 `.rove` 中 B 类)与 run-owned(`runs/<id>/`)根,
  满足计划要求的"明确区分 config、data、state、cache、workspace-owned、
  run-owned"。
- 目录创建时 unix 权限 0700(对齐 `user_config/writer.rs:87-115` 现有策略)。

**明确不迁移**:用户配置根保持 `~/.rove` 不动。它是纯用户级、无项目内
legacy 副本可发现,搬动只会破坏所有既有安装的 provider catalog,且计划
的目标边界(项目目录瘦身)不要求它搬家。

## 3. Workspace 存储身份(Implemented)

- `storage_key = hex(stable_hash("{canonical_root_key}|{kind}"))[..16]`。
- `canonical_root_key` 复用 `project_trust.rs:1226-1231` 的规范化
  (Windows 小写、反斜杠归一、剥 `//?/` 前缀),输入先经 `canonicalize()`
  归并 symlink/reparse point。
- **与 trust identity digest 的区别**:trust digest 额外混入 dev/ino
  (unix) 或 creation_time(Windows)(`project_trust.rs:1122-1148`),用于
  检测目录被替换;storage key 只按规范化路径 + kind,保证"同一 workspace
  的不同入口(cwd、API 绝对根、Desktop)得到同一目录"。两者是不同用途的
  不同摘要,不共用。
- 不同 canonical 路径必得不同 key(hash 截断 16 hex,碰撞概率可忽略,
  并由 `workspace.json` 二次校验兜底:记录 `canonical_root` 与 `kind`,
  打开时不匹配即 typed error,不静默混用)。
- workspace 被移动/重命名 → 新 key → 视为新 workspace(等价"只有新用户
  目录"场景);旧数据留在旧 key 目录与旧项目 `.rove/`,迁移命令按
  legacy 发现处理。这是显式记录的边界,不做 move-following。

## 4. 配置与入口合同(Implemented)

### 4.1 `StateConfig` 语义

- `state_dir` / `sqlite_path` 默认值从 `".rove"` 改为**空路径哨兵**
  (`PathBuf::new()`)= "未配置 → 走共享契约"。serde 兼容:旧配置文件未写
  `[state]` 段时行为等价"未配置";显式写了 `.rove` 的旧配置(如
  `.rove/config.example.toml` 引导出的)**继续按现行语义工作**(相对 →
  workspace 根,绝对 → 需 `allow_external_paths` 或 user-config 已加载),
  但 `rove state paths` 会给出 `legacy_explicit_state_dir` 诊断。
- `memory.session_dir` / `durable_dir` 同样改哨兵 →
  `<user ws>/memory/sessions`、`<user ws>/memory`。
- `tool.mcp_config_path` 默认同样改哨兵 → `<user ws>/mcp_servers.json`;
  显式配置(如团队在 `.rove/config.toml` 指向项目内共享 catalog)继续按
  现行语义工作。`workspace_bounded_mcp_config_path`(`config.rs:816-833`)
  对默认值改为以 `<user ws>` 为边界,产品 Settings/MCP 管理面随之写入用户
  目录、不再触碰项目目录;`registry.rs:101-120` 的 `mcp_processes` 能力门
  与 `digest_mcp_configuration`(`project_trust.rs:1026-1039`)改为摘要
  **生效 catalog 路径**的内容——迁移是逐字节复制,内容不变则既有授权
  digest 不变,既不自动授予也不静默失效。
- 新增 `AppConfig` 解析结果投影(放 `ConfigSourceSummary` 或伴生结构):
  `data_root`、`workspace_storage_key`、`resolved_state_dir`、
  `resolved_sqlite_path`、`resolved_memory_*`、`legacy_state_present`,
  供 CLI/API 诊断复用,不产生第二份 authority。
- 校验规则:`validate_workspace_paths` 继续约束**显式配置**的路径;契约
  派生的默认路径由构造保证在 `<data_root>` 之下,不适用 workspace 边界
  检查。`workspace_bounded_durable_memory_dir`(`config.rs:807-813`)改为
  对契约目录做边界(在 `<user ws>` 内),产品 Memory 视图语义不变。
- `state.lazy_migration` 字段保留(现行语义是"索引为空时从 artifacts 惰性
  导入",`store.rs:104-119`),不新增含义,向后兼容读取。

### 4.2 环境变量

| 变量 | 语义 | 现状 |
|---|---|---|
| `ROVE_DATA_ROOT` | data_root 覆盖,必须绝对,fail-closed | **新增** |
| `ROVE_CONFIG_ROOT` | 用户配置根覆盖 | 不变 |
| `ROVE_STATE_DIR` / `ROVE_STATE_SQLITE` | 显式 state 覆盖(进入 env layer,现行 workspace 边界校验照旧) | 不变 |
| `ROVE_PROJECT_TRUST_STORE`、`ROVE_TRUSTED_WORKSPACES` | 不变 | 不变 |

测试一律用 `ROVE_DATA_ROOT`/`ROVE_CONFIG_ROOT` 显式绝对路径或
`load_with_authorities` + `from_root` 构造(现行测试惯例,
`tests/api.rs:4194-4200,4308-4366`),绝不触真实 home。

### 4.3 共享解析模块位置

新增 `apps/bootstrap/src/user_state/`(rove-app-bootstrap 拥有产品配置与
装配,且 `UserConfigPaths`、`operator_state_base` 已在此层):
`UserStateRoots`(根解析)、`WorkspaceStorageIdentity`(key/digest)、
`StatePathContract`(单 workspace 布局解析、redact 助手)。rove-runtime
不发现 home 目录——继续只接收已解析的绝对路径(`Workspace.state_dir`、
`MemoryPaths`、`StateStore` 构造参数),维持
`rove-runtime <- rove-app-bootstrap` 依赖方向。

### 4.4 `Workspace` 与 Engine

- `Workspace.state_dir` 字段语义收敛为"由产品路径契约注入的已解析 state
  目录"。`Workspace::detect/task/open_folder/open_repo` 的 `.rove` 默认
  **保留**,但文档改为"legacy/embedding 默认",兼作迁移命令的 legacy
  发现锚点;所有生产入口在构造后立即以契约结果覆盖(现状已是
  `workspace.state_dir = config.state_dir()` 模式,`apps/cli/src/cli/runtime.rs:317-325`、
  `apps/api/src/lib.rs:359-363,3734-3745`)。
- `Engine::new` 的 `cwd.join(".rove")` 回退仅直接嵌入可用;改为文档标注
  并加 debug 断言路径(不改变行为),生产装配不经过它。
- 生产入口接线清单(实现时逐一核对并加断言测试):
  CLI `runtime.rs`/`sessions.rs`/`state.rs`/`config.rs`(dump-config 的
  `resolved_paths`)、API `serve_with_shutdown`/`embedded_api_state`/
  `state_store_for_parts`/`rebased_workspace_config`/product store
  路径/task base、bootstrap `assembly.rs`、M1 import
  (`product/migration.rs:294,489` 改为契约目录 + legacy `.rove` 双查)。

### 4.5 CLI/API 诊断

- API `lib.rs:3525` 与 CLI 的 `~/.rove/config.toml` 文案核对更新。
- API/Web 不新增路径判断端点;需要展示时复用共享 serde 结构(本期仅 CLI
  使用,API 侧现有 runtime 诊断若含绝对路径则套 redact)。
- trace/report/错误中的用户绝对路径经契约 `redact()`(以 data_root /
  config_root / home 前缀替换为 `<rove-data>/…` 形式);CLI 本机
  stdout(`rove state paths`)面向操作者,显示真实路径。

## 5. 当前实现迁移合同（Implemented）

### 5.1 CLI 表面

```
rove state paths                      # 查看解析结果与 workspace 身份(pretty JSON)
rove state migrate                    # dry-run(默认),输出计划报告,零写入
rove state migrate --apply            # 执行迁移
rove state migrate --apply --on-conflict backup-target   # 冲突时备份目标后复制
rove state migrate --prune-legacy     # 仅在成功回执存在时删除已迁移的 legacy 文件(需 --apply)
```

- 输出风格对齐现行:`trust` 的 pretty JSON(`cli/trust.rs:38-71`)与
  `provider migrate` 的报告 JSON + `code: message` 错误
  (`cli/provider.rs:40-46`)。JSON 走 stdout,日志走 stderr,退出码稳定:
  0 = 成功/干净 dry-run;非 0 + `state_migration_conflict` /
  `state_migration_locked` / `state_migration_incomplete` /
  `state_migration_invalid_source` 等 typed code。
- dry-run 硬约束:不创建任何目录/文件、不打开写事务、不启动 Provider/MCP、
  不改 Trust、不调用模型。SQLite 源以只读连接读取 schema 版本用于风险
  报告(只读连接在允许范围,不产生写)。

### 5.2 计划报告(dry-run 与 apply 共用结构)

```json
{
  "schema_version": 1,
  "applied": false,
  "workspace": {"root": "…", "kind": "repo", "storage_key": "…", "target_dir": "…"},
  "source": {"dir": "…", "layout": "legacy_project_local", "present": true,
             "receipt": null},
  "plan": {"files": [{"path": "state.sqlite", "class": "state_sqlite", "bytes": 40960}, …],
           "total_bytes": 123456, "skipped": [{"path": "config.toml", "reason": "project_config_stays"}]},
  "conflicts": [{"path": "…", "reason": "target_differs", "resolution": "keep_target"}],
  "risks": ["sqlite_schema_newer_than_runtime", "memory_index_temp_residual", "…"],
  "journal": {"path": "…", "status": "none|in_progress|complete"},
  "legacy_disposition": "kept"
}
```

文件分类 class 即 §1.1-C 表;`unknown`(无法归类文件)按原样复制并显式
列出,不静默丢弃。遍历有界:最大深度、最大条目数、总量上限(默认 2 GiB,
可 `--max-bytes` 覆盖),拒绝跟随源内 symlink(记录为 risk)。

### 5.3 幂等与 journal

- 幂等基准是**逐文件内容 hash**,不是事件计数:目标存在且 sha256 相同 →
  skip;不同 → conflict;不存在 → 复制后写 journal 行
  `{seq, path, class, bytes, source_sha256, target_sha256, outcome}`。
- journal(`<target>/.migration/journal.jsonl`)在 SQLite 原子 rename 前先
  同步 `prepared` source/target digest，再追加最终 outcome；因此进程在
  rename 后、最终行前退出也能无冲突恢复。完成标记
  `<target>/.migration/migration.json`(版本、时间、清单摘要)仅供
  观测与诊断；单行 journal 损坏不阻塞重跑(重新推导计划，hash 比较保证
  不重复、不覆盖)，另有源侧回执 `.rove/.rove-migration-receipt.json`
  (记录 target、清单 digest、时间),用于"两边都有数据"时判定已迁移、
  不重复提示。
- 进程任意阶段退出后重跑 `--apply`:已复制文件 skip,未完成文件重做;
  SQLite 快照目标已存在且 hash 一致 → skip。不产生重复 memory/Artifact/
  ProductStore 记录(复制语义,不重放事件)。

### 5.4 SQLite 快照

`state.sqlite` / `product.sqlite` 用 `VACUUM INTO`(只读打开源,写一致
单文件快照,规避 WAL/wal-shm 拷贝陷阱);源被锁或不可读 → typed error。
`state.sqlite` 历史上保存的绝对 run/trace/task/report 路径在临时快照内
事务化重定位到目标 state 根，旧根以外的值不改；重定位、`quick_check`、
文件同步与 prepared journal 全部成功后才原子 rename，因此 prune 后按原
`run_id` resume 不会回指 legacy `.rove`。
目标打开时由现行 `apply_migrations`(StateIndex `index.rs:1944-1986`、
ProductStore `schema.rs:514-558`)自然完成 schema 升级;**高于当前版本**
的源库在迁移计划阶段即列为 risk 并在目标打开时按现行语义拒绝
(fail-visible,不降级不覆盖)。

### 5.5 冲突与恢复

- 默认 `keep_target`:目标保留,源不动,条目列入 `conflicts`,迁移可部分
  成功但报告非干净;`--on-conflict backup-target`:目标文件移入
  `<target>/.migration/conflicts/<seq>/` 后复制源。任何策略都不静默覆盖。
- 并发:`<target>/.migration/lock` flock(对齐 `user_config/writer.rs:117-150`
  的 5s 超时模式);持锁失败 → `state_migration_locked`。
- 回滚:复制语义 + 默认 `legacy_disposition: kept`,失败时旧数据始终可读;
  回滚 = 删除/忽略 target(或重跑修复)。SQLite 临时快照先重定位索引路径、
  quick-check、sync 并记录 prepared digest，再原子发布；即使最终 outcome
  尚未写入，重跑也能验证目标而不会留下“成功但 resume 找不到事实”的半状态。
- `--prune-legacy` 先写临时完成回执，再逐文件复核；仅删除已迁移且验证
  通过的文件。`.rove/config.toml`、`.env` 等项目配置永不删除；默认迁移的
  `mcp_servers.json` 属于用户 workspace state，可以在显式 prune 时删除。
- `cleanup` / `repair` 与迁移是三个独立动作:`state cleanup` 现行 TTL
  语义不变(`index.rs:1409-1437`),`state repair` 现行索引重建语义不变
  (`store.rs:242-252`),互不冒充。

### 5.6 场景合同与证据范围

`apps/bootstrap/tests/state_migration.rs` 的 23 例确定性测试覆盖 fresh、
legacy、新/旧两侧共存、同/异内容、普通与 prepared-SQLite 中断重试、损坏
journal/SQLite、锁竞争、无效 data root、memory/Artifact/MCP/ProductStore
冲突、source/target/metadata symlink、一次性与部分 prune，以及真实旧 run
在 prune 后的精确 resume。Trust 不变由 MCP digest 回归证明；`state repair`
和 `state cleanup` 继续由既有 Runtime 测试负责。真实操作系统 ACL 拒绝、
macOS/Linux 执行、外部 Provider/MCP 和安装版 Desktop 属于未运行 gate，
状态只在 `VERIFICATION.md` 记录，不用设计清单冒充实测证据。

## 6. 不变量与明确不改的东西

1. **Trust 不动**:迁移不读不写 trust 库、不授予 capability;B 类项目来源
   的授权语义原样。`mcp_processes` 的 capability digest 随生效 catalog
   路径解析其内容:逐字节复制使既有授权 digest 保持不变,迁移既不自动
   授予也不静默失效;显式项目 catalog 的 digest 语义不变。
2. **事件身份与精确 resume**:`trace.jsonl`/`task_state.json`/`report.json`
   迁移中逐字节复制,内容零改写;`RuntimeIdentity`(workspace root+kind
   指纹,`foundation/runtime_identity.rs:113-146`)输入不变,迁移后旧 run
   必须仍可精确 resume(测试断言)。
3. **指针字符串不改写**:checkpoint 内嵌 `.rove/memory/sessions/<id>.md`
   等指针(`state/artifacts.rs:790-791`)是记录性字符串,运行时一律经
   `MemoryPaths` 绝对路径解析读取;迁移不改写以保 2。新增测试证明无任何
  运行时把该字符串当本地相对路径用。
4. **canonical events / ToolRegistry snapshot / approval policy / Provider
   snapshot** 不因目录切换改变。
5. **已完成的副作用不重放**:复制语义天然满足;resume 的 trace-tail 对账
   (`state/reconcile.rs:49-63`)照旧。
6. **secret-free**:迁移报告、`state paths` 的 API/Web 暴露面、trace、
   report 不出现原始 key / Authorization / 环境值 / 未脱敏用户绝对路径
   (CLI 本机 stdout 除外,面向操作者)。
7. **bench 不变**:显式本地布局属一次性产物。
8. **Desktop**:`embedded_api_state` 改为契约解析(签名保留 state_dir
   覆盖能力);`%APPDATA%\Rove\state\product.sqlite` 成为额外 legacy 来源
   记录在案;Desktop 完整安装旅程本就 Not Run,不在本任务补验。

## 7. 实现顺序与当前落点

1. 仓库根 `IMPLEMENTATION_LOG.md`(基线、计划、真实验证命令)已建立。
2. `apps/bootstrap/src/user_state.rs` 契约模块 + 单测(根解析 fail-closed、
   storage key 归并/隔离、redact)。
3. `AppConfig` 集成(哨兵默认:state/sqlite/memory/mcp catalog、解析投影、
   校验、`workspace_bounded_mcp_config_path` 与 trust digest 的生效路径
   接线、rebase、dump-config)。
4. 入口接线(§4.4 清单)+ 接线断言测试。
5. 迁移引擎(`apps/bootstrap/src/state_migration.rs`)+ §5.6 关键负向测试 +
   Windows 可复跑脚本(`scripts/`)。
6. CLI `state paths` / `state migrate` + 输出格式测试。
7. 文档:`STATE_LAYOUT_AND_MIGRATION.md`(新)、`docs/runtime/`
   (subsystems/architecture/implementation-guide/implementation-status/
   acceptance-matrix)、`docs/ONBOARDING.md`、`README.md`、
   `.rove/config.example.toml`、`SUMMARY.md`、`VERIFICATION.md`、
   `DIFF_SUMMARY.md`。
8. 验证阶梯已按 `IMPLEMENTATION_LOG.md` 执行；Windows smoke、Web、外部
   Provider/MCP 和 Desktop 安装 gate 的状态在 `VERIFICATION.md` 中单独列出。

## 8. 本设计不做

会话全文搜索、Context Inspector、Review、managed worktree、后台任务中心、
插件市场、向量 RAG、第二套 Agent loop / 事件 / 队列 / state authority、
迁移后自动授权、`~/.rove` 用户配置根搬家、workspace move-following。
