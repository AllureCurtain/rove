# Rove 协议 v1

`rove-protocol` 是 workspace 的叶子 crate：它只依赖 `serde` 与 `ulid`，不依赖 tokio、axum、utoipa，也不依赖任何其他 rove crate。这条约束是这个 crate 存在的理由——只需要解析一个 run id 或者匹配一个 run status 的消费方，不应该被迫链接一个异步运行时。

验收命令：

```bash
cargo tree -p rove-protocol            # 24 个依赖，无 tokio/axum/utoipa/reqwest
cargo tree -i tokio -p rove-protocol   # 空
cargo tree -i axum  -p rove-protocol   # 空
```

## 1. 标识符

四个 ULID newtype，wire 形式就是裸 ULID 字符串（`"01J8Z…"`），不是对象。

| 类型 | 含义 |
|------|------|
| `SessionId` | 会话，跨多个 job |
| `JobId` | 一次任务提交 |
| `RunId` | 一次引擎主循环执行 |
| `CallId` | 一次工具调用 |

每个都提供 `new()`（生成，ULID 单调可排序）、`Display`、`FromStr`（失败返回描述性 `String`，不 panic）、`Default`。

历史路径保持可用：`rove_runtime::types::{SessionId, JobId, RunId}` 与 `rove_core::CallId` 都是对本 crate 的 re-export，因此**全部 1718 处调用点未作任何修改**。

OpenAPI schema 不在本 crate 里声明，而是在 `apps/api` 的使用处以 `#[schema(value_type = String, format = "ulid")]` 挂载——这正是本 crate 得以不依赖 utoipa 的原因。

## 2. 生命周期枚举

全部 `snake_case`。改名即破坏性变更。

| 类型 | 取值 |
|------|------|
| `RunStatus` | `init` `running` `done` `error` `cancelled` `interrupted` |
| `ApprovalPolicy` | `ask` `auto` `never` |
| `RunMode` | `normal`（默认）`review` |
| `ApprovalDecision` | `approve` `reject` |

`RunMode` 的 `Default` 是 `Normal`：缺失字段永远不能升级为 review 权限。`lifecycle.rs` 中有一个测试把这些 wire 拼写逐个钉住，使得一次 rename 先在本 crate 失败，而不是等到线上客户端或已落盘的 artifact 上暴露。

## 3. 版本与信封

`PROTOCOL_VERSION: u32 = 1`。

| 版本 | 随哪期发布 | 变更 |
|------|-----------|------|
| 1 | Phase 4 | 首个显式版本化信封；标识符、生命周期枚举、stream event 上的 `v` 字段 |

升版规则：只有当变更会让**旧客户端误读新服务端**时才升。新增可选字段、或新增一个客户端本就应当跳过的 variant，不需要升版。

`/jobs/{id}/events` 的每一帧都以版本号作为首字段：

```
id: 1
event: run_started
data: {"v":1,"type":"run_started","run_id":"01J…","job_id":"01J…","user_message":"…"}
```

信封 `Versioned<T>` 用 `#[serde(flatten)]` 承载 payload 而非嵌套，这一点是刻意的：**versioning 之前写的客户端仍然在原位置找到 `type` 和全部事件字段**，只是多看到一个它会忽略的 `v`。反向兼容同样成立——`v` 的 serde default 是 `PROTOCOL_VERSION`，因此一条 `v` 字段出现之前录制的帧仍然能反序列化。

`v` 只加在 SSE 出口（`apps/api` 的 `sse_event`），不加在 `trace.jsonl`。trace 有自己的 schema 版本，不应该继承一个 wire 层面的关注点。

## 4. 与计划的分歧

Phase 4 原计划的三条验收标准均与真实代码不符，已按 §0.3 规则记录在实施计划文档中。简述：`apps/api/src/lib.rs` 里没有 DTO 可外移（5802 行、62 个 handler、仅 1 个 `pub struct`）；把 crate 放在 `rove-models` 之下并不能避开 tokio（`models/Cargo.toml` 自身就依赖 tokio）；desktop 并未复制 DTO（它整体依赖 `rove-api`）。

实际落地的形态比原计划更强：真正零 tokio/零 axum 的叶子 crate，且因为采用 re-export 而非搬迁+改调用点，迁移成本为零。
