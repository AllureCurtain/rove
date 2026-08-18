# IMPLEMENTATION_LOG — user-state-migration worktree

## 基线与边界

- Worktree: `.worktrees/user-state-migration`
- Branch: `feature/user-state-migration`
- 基线提交: `5fe9d70` (`docs: add next-round productization and parallel task plans`)，
  基于 main `f6676d1` (PR #33)
- 开始状态: clean
- 平台: Windows 10.0.26200 x64，PowerShell，Asia/Shanghai
- 计划: `docs/plans/2026-08-16-user-state-directory-migration.md`
- 设计: `docs/design/2026-08-16-user-state-directory-migration-design.md`
- 明确未读取、修改、搬运或评审 `.worktrees/read-only-review`；总纲
  `docs/plans/2026-08-16-next-productization-round.md` 仅作上下文。

## 实施记录

- 2026-08-16: 核对 `UserConfigPaths`、`AppConfig`、Workspace、Runtime
  state/memory、ProductStore、Project Trust、CLI/API/Web/Desktop/bench 路径；
  完成逐路径 owner/authority/lifecycle/sensitivity/migration 设计并获确认。
- 2026-08-16 至 2026-08-17: 实现 `UserStateRoots`、稳定 workspace storage
  key、`workspace.json` marker、跨平台 data root 和绝对 `ROVE_DATA_ROOT`
  override；state/sqlite/memory/MCP 默认路径改为契约哨兵解析，显式配置保持
  历史语义。
- 接通 CLI exec/REPL/TUI/sessions/state/trust、API standalone/embedded、
  per-job rebind、ProductStore、M1 import、MCP Settings/registry 与 benchmark
  的共享 `AppConfig` 解析结果。默认 MCP catalog 进入用户 workspace state，
  legacy fallback 和 `mcp_processes` capability digest 保持连续，迁移不授予
  Trust。
- 实现 `rove state paths` 与 dry-run 默认的 `rove state migrate`：有界清单、
  sha256 幂等、SQLite `VACUUM INTO`、keep/backup 冲突、workspace/global 锁、
  journal/receipt、显式 prune、symlink/reparse 边界和稳定错误码。
- 2026-08-17 复核修正: `state.sqlite` 的 run/trace/task/report 索引保存绝对
  路径，单纯复制会在 prune 后回指 `.rove`。新增
  `StateIndex::rebase_artifact_paths`，在临时快照内事务化重定位旧根下路径；
  `prepared` journal 在原子 rename 前同步，关闭 rename 后、最终 outcome 前
  崩溃的冲突窗口。Windows 8.3、大小写、`\\?\` 与扩展 UNC 路径统一处理。
- 强化 `.migration`、`conflicts`、lock、journal 和临时文件的类型/symlink
  检查；Windows `ERROR_SHARING_VIOLATION` / `ERROR_LOCK_VIOLATION` 映射为
  `state_migration_locked`。
- 2026-08-18 最终 M1 复核：server-owned workspace discovery 在 contract
  尚未物化时继续读取 legacy `.rove`，在 `state.sqlite` 或 `runs/` 出现后
  切换到用户级目录；API singleton artifact 校验仅接受带有效
  `workspace.json` marker 的 contract state。新增两条聚焦回归并随完整
  workspace 门禁通过。
- 2026-08-18 最终 MCP 复核：Product Settings 首次写入先校验请求并建立
  marker-bound contract layout，在目标 catalog 锁内仅一次提升 legacy servers；
  已存在 contract 始终胜出，后续 legacy 修改不会覆盖或复活。不存在 catalog
  的只读查询保持零目录写入，mutation 同时失效 legacy/contract health cache。
- `integration-smoke.ps1` 与 `provider-integration.ps1` 显式把
  `ROVE_DATA_ROOT` 绑定到各自 disposable integration root，避免默认
  ProductStore 或 contract state 污染真实用户 profile。
- Windows smoke 的原生命令 stderr 处理兼容 Windows PowerShell 5.1 与
  PowerShell 7，两个宿主均通过 19/19 场景。
- 同步 `docs/runtime/`、根 README/ONBOARDING、示例配置和
  `STATE_LAYOUT_AND_MIGRATION.md`；交付摘要与验证清单见 `SUMMARY.md`、
  `VERIFICATION.md`、`DIFF_SUMMARY.md`。

## 已执行的真实验证

```text
cargo fmt --all --check
  PASS

cargo test -p rove-app-bootstrap --lib
  PASS: 82 passed (最终 workspace 运行)

cargo test -p rove-app-bootstrap --test state_migration -- --nocapture
  PASS: 23 passed

cargo test -p rove-api --lib canonical_runtime_index_rejects_an_invalid_contract_marker -- --nocapture
  PASS: 1 passed

cargo test -p rove-api --lib marker_bound_contract_state_runtime_binding_is_verified -- --nocapture
  PASS: 1 passed

cargo test -p rove-app-bootstrap --lib state_discovery_falls_back_to_legacy_until_contract_state_materializes -- --nocapture
  PASS: 1 passed

cargo test -p rove-runtime --lib state::index::tests::sqlite_corruption_maps_to_invalid_data -- --nocapture
  PASS: 1 passed

cargo test -p rove-runtime --lib tools::mcp_config::tests
  PASS: 6 passed

Product MCP focused API regressions
  PASS: 5 passed, including first-write legacy promotion

scripts/state-migration-smoke.ps1 -CargoRoot ""
  PASS: 19 assertions (PowerShell 7)

powershell -ExecutionPolicy Bypass -File scripts/state-migration-smoke.ps1 -CargoRoot ""
  PASS: 19 assertions (Windows PowerShell 5.1)
```

最终门禁补充：

```text
cargo clippy --workspace --all-targets -- -D warnings
  PASS

cargo test --workspace -j 1
  PASS: all packages, integration tests, and doc tests; 0 failures
  Final post-M1 run: 2026-08-18, exit 0

cd apps/web; pnpm test
  PASS: 36 files, 241 tests

cd apps/web; pnpm typecheck
  PASS

cd apps/web; pnpm build
  PASS

cd apps/web; pnpm test:e2e
  PASS: 56 passed, 5 gated real-API cases skipped

scripts/integration-smoke.ps1 -ApiAddr 127.0.0.1:18787 -WebPort 13000 -IntegrationRoot <temporary-root>
  PASS: 5/5 live real-API Playwright cases; isolated ROVE_DATA_ROOT/ProductStore

git diff --check
  PASS
```

第一次 `cargo test --workspace` 并行运行触发 Windows linker 共享 PDB 限制
(`LNK1318: PDB LIMIT (12)`)，没有 Rust 编译或断言错误；随后以 `-j 1`
串行重跑并取得上述完整 PASS。为适配新增“数据根不得位于 workspace 内”安全
边界，CLI/TUI 与 REPL 集成测试夹具改为使用独立临时数据根，并显式断言用户级
state layout。

最终 `fmt`、`clippy --workspace --all-targets`、`test --workspace`、Windows
smoke、Web 单元/type/build/mocked E2E、live `local-full`、文档卫生和
`git diff --check` 的结果已同步到 `VERIFICATION.md`。外部 Provider、真实
第三方 MCP、安装版 Desktop、macOS/Linux 打包、Windows ConPTY 和
soak/stress 不在本地默认迁移门禁内，未运行时明确标为
Not Run/Unverified。
