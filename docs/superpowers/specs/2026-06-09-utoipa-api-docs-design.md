# Utoipa API Docs Design - 2026-06-09

本文定义 `rove` 接入正式 OpenAPI/Swagger 文档的设计方向。目标是让当前 HTTP API
从“运行时文档里列路由”升级为“代码生成的、可测试的、可浏览的 API Reference”，并参考
`D:/Study/project/mega` 中 `utoipa`、`utoipa_axum`、`utoipa_swagger_ui` 的组织方式。

这是一份设计 spec，不是实现计划。后续实现应在
`D:/Study/project/agent/rove/.worktrees/utoipa-api-docs` 对应的隔离 worktree 中进行。

## Suggested /goal Objective

后续可以使用这个目标启动开发：

> Based on `docs/superpowers/specs/2026-06-09-utoipa-api-docs-design.md`, implement formal OpenAPI documentation for rove's HTTP API using `utoipa`, `utoipa_axum`, and `utoipa_swagger_ui`: generate `/api/openapi.json`, expose `/swagger-ui`, document all current API routes and schemas, include bearer-token security metadata, add regression tests for the generated spec and Swagger UI route, and update runtime documentation, all inside `.worktrees/utoipa-api-docs`.

## Current State

`rove` 当前 HTTP API 集中在 `src/interfaces/api/mod.rs`，由普通 `axum::Router` 手写路由：

- `POST /providers/test`
- `POST /jobs`
- `GET /jobs/{job_id}/events`
- `GET /jobs/{job_id}/state`
- `POST /jobs/{job_id}/cancel`
- `POST /jobs/{job_id}/approvals/{call_id}`
- `POST /jobs/{job_id}/inputs/{input_id}`
- `GET /runs`
- `GET /runs/{run_id}/report`

请求和响应 DTO 也基本集中在同一文件，例如 `CreateJobRequest`、`ProviderTestRequest`、
`CreateJobResponse`、`JobStateResponse`、`ListRunsResponse` 和 `ProviderTestResponse`。
运行时文档 `docs/runtime/subsystems.md` 只维护了人工路由列表；
`docs/runtime/integration-testing.md` 提供手工 smoke 示例；项目目前没有 OpenAPI JSON、
Swagger UI 或 schema 回归测试。

相比之下，`mega` 的正式 API 文档做法包括：

- 用 `utoipa::OpenApi` 定义集中 `ApiDoc`。
- handler 上使用 `#[utoipa::path]` 描述路径、参数、请求体和响应。
- DTO 上派生 `utoipa::ToSchema`。
- 通过 `utoipa_swagger_ui::SwaggerUi` 挂载浏览器文档。
- 在 `mega/mono` 中使用 `utoipa_axum::OpenApiRouter` 让路由注册和 OpenAPI 聚合绑定。

`rove` 应采用更接近 `mega/mono` 的路线，而不是只在旁边手写一个独立 JSON endpoint。

## Design Goals

1. **正式 API Reference**
   运行时服务应提供机器可读的 `/api/openapi.json` 和浏览器可读的 `/swagger-ui`。

2. **路由和文档绑定**
   新增或调整 API 时，应尽量通过 `utoipa_axum::OpenApiRouter` 让路由注册和 OpenAPI
   path 注册一起发生，降低“实现有了但文档漏了”的概率。

3. **保留当前业务 API**
   第一版不改变现有业务路径、请求语义、响应语义、SSE 行为或安全中间件语义。

4. **安全边界清楚**
   OpenAPI 要表达 bearer token 安全要求、CORS/rate limit 的存在，以及 provider key
   通过环境变量名引用。文档和 schema 都不引入真实 API key 字段。

5. **可测试**
   生成出来的 OpenAPI spec 本身要有回归测试，防止路径、schema 或安全信息悄悄丢失。

6. **适合面试展示**
   Swagger UI 应能直观展示 `rove` 的 job lifecycle、provider test、approval/input
   和 run report API，使项目看起来是一个完整 runtime，而不是只有 CLI 原型。

## Non-Goals

第一版不做以下内容：

- 不重命名或版本化现有业务路径，例如不把所有接口迁移到 `/api/v1/*`。
- 不重构 API handler 的业务逻辑。
- 不引入用户系统、多租户、OAuth 或分布式限流。
- 不把 SSE event payload 设计成新的公共事件协议；只记录当前实际返回结构。
- 不要求 Swagger UI 为业务 API 自动获取、保存或刷新 token。
- 不生成客户端 SDK。

## Architecture

### Module Layout

新增 `src/interfaces/api/docs.rs`，专门承载文档相关定义：

- `ApiDoc`：`#[derive(utoipa::OpenApi)]` 的 OpenAPI 根定义。
- tag 常量：
  - `JOBS_TAG`
  - `JOB_EVENTS_TAG`
  - `APPROVALS_TAG`
  - `RUNS_TAG`
  - `PROVIDERS_TAG`
- OpenAPI `info`、`tags`、`components` 和 `security` 配置。

`src/interfaces/api/mod.rs` 继续保留现有 runtime API 逻辑，但增加：

- `mod docs;`
- DTO 的 `utoipa::ToSchema` 派生。
- handler 的 `#[utoipa::path]` 注解。
- router 构建从普通 `Router::new()` 调整为基于 `OpenApiRouter::with_openapi(ApiDoc::openapi())`
  的组合方式。

### Router Shape

`router(state: ApiState) -> Router` 仍保持对外签名不变，避免影响 tests 和 `rove-api` binary。
内部流程调整为：

1. 用 `OpenApiRouter::with_openapi(ApiDoc::openapi())` 创建文档感知 router。
2. 用 `routes!(handler)` 或等价方式注册现有 9 条 API。
3. 对业务 API router 挂上现有 `api_security` middleware 和 `state`。
4. `split_for_parts()` 得到普通业务 `Router` 和生成的 OpenAPI spec。
5. 在业务 router 外层合并 `SwaggerUi::new("/swagger-ui").url("/api/openapi.json", api)`。

这样最终仍返回普通 `axum::Router`，但 API 路由、OpenAPI paths 和 Swagger UI 来自同一个注册过程。
文档端点只暴露静态规范和 UI；通过 Swagger UI 调用业务 API 时，仍遵循业务 API 的 bearer token、
CORS 和 rate-limit 规则。

### Public Documentation Endpoints

第一版暴露：

- `GET /api/openapi.json`
- `GET /swagger-ui`

业务 API 路径保持不变。`/api/openapi.json` 只作为文档规范路径，不代表业务 API 已整体迁移到
`/api/*` namespace。

## API Coverage

OpenAPI 第一版覆盖当前所有 HTTP API：

| Method | Path | Tag | Notes |
|---|---|---|---|
| POST | `/providers/test` | Providers | 测试 provider profile 和 model visibility |
| POST | `/jobs` | Jobs | 创建 job，可带 per-request provider profile |
| GET | `/jobs/{job_id}/events` | Job Events | SSE event stream，支持 `after` query 和 `Last-Event-ID` |
| GET | `/jobs/{job_id}/state` | Jobs | 查询 live 或 persisted job state |
| POST | `/jobs/{job_id}/cancel` | Jobs | 取消 active job，终态 job 幂等返回当前状态 |
| POST | `/jobs/{job_id}/approvals/{call_id}` | Approvals | 提交 tool approval decision |
| POST | `/jobs/{job_id}/inputs/{input_id}` | Approvals | 回答 `request_input` pending input |
| GET | `/runs` | Runs | 列出 indexed run summaries，支持 `limit` |
| GET | `/runs/{run_id}/report` | Runs | 获取 persisted run report |

错误响应第一版统一记录常见状态码：`400`、`401`、`403`、`404`、`409`、`429`、`500`。
如果当前 `ApiError` 没有结构化 JSON error body，则 OpenAPI 中先以 plain text 或 unspecified body
描述，避免文档承诺不存在的错误 DTO。

## Schema Coverage

以下 DTO 应派生或注册 `ToSchema`：

- `PendingApprovalResponse`
- `PendingInputResponse`
- `CreateJobRequest`
- `ProviderProfileRequest`
- `ProviderTestRequest`
- `CreateJobWorkspace`
- `CreateJobWorkspaceKind`
- `SubmitApprovalRequest`
- `SubmitInputRequest`
- `JobStreamEvent`
- `CreateJobResponse`
- `JobStateResponse`
- `ListRunsResponse`
- `RunSummaryResponse`
- `ProviderTestResponse`

如果下游类型已经可派生 `ToSchema`，优先在原类型上添加派生；如果属于核心 runtime 类型且添加
`utoipa` 派生会造成不合理耦合，则在 API docs 层使用 schema aliases 或 wrapper DTO。
这类例子可能包括 `ApprovalPolicy`、`ApprovalDecision`、`RunStatus`、`StreamEvent` 和
`RunReport`。

设计原则是：优先文档化真实 API 结构；只有当真实类型不适合直接暴露给 `utoipa` 时，才引入文档
专用 schema。

## Security Documentation

OpenAPI components 增加 bearer token security scheme：

- scheme name: `BearerAuth`
- type: HTTP bearer
- bearer format: token

业务 API paths 默认标记可使用 `BearerAuth`。实际是否要求 token 仍由当前运行时配置
`api.token_auth` 决定；OpenAPI 负责告诉调用方这个 runtime 支持并可能要求 bearer token。
`/swagger-ui` 和 `/api/openapi.json` 不放宽业务 API 的安全策略，它们只提供文档入口；需要鉴权的
业务请求仍由 `api_security` 拦截。

Provider 相关 schema 只包含：

- `name`
- `api_base`
- `api_key_env`
- `model`
- `models_endpoint`

不新增 `api_key`、`secret`、`token` 等直接承载密钥的请求字段。`ProviderTestResponse` 只暴露
`key_env` 和 `key_present`，不暴露真实密钥值。

## Testing Strategy

实现前先补 API docs 回归测试，重点验证生成结果而不是只验证 handler 能编译：

1. `GET /api/openapi.json` 返回 `200 OK`，body 是 JSON，并包含 `openapi`、`info` 和 `paths`。
2. `paths` 包含当前 9 条 API。
3. `components.schemas` 包含核心请求/响应 schema。
4. `components.securitySchemes.BearerAuth` 存在。
5. `GET /swagger-ui` 返回成功或重定向到 Swagger UI 资源。
6. 生成的 spec 文本不包含测试用真实 provider token，只出现 `api_key_env`、`key_present`
   等安全字段。

测试可放在现有 `tests/api.rs`，复用 `router(ApiState::new(...))` 和 `tower::ServiceExt::oneshot`。
如果 Swagger UI 的 exact status 受 crate 版本影响，测试只要求它不是 `404`，并检查响应可访问。

## Documentation Updates

实现完成后更新：

- `docs/runtime/subsystems.md` 的 API And Security 小节，加入 `/api/openapi.json` 和 `/swagger-ui`。
- `docs/runtime/integration-testing.md`，补充如何启动 `rove-api` 后查看 Swagger UI。
- 根 `README.md` 或 runtime docs index，如果已有 API 文档入口，则链接到 runtime API reference。

可以新增 `docs/runtime/api-reference.md`，但第一版不必手写完整接口文档；OpenAPI spec 是主来源。

## Risks And Mitigations

1. **`utoipa_axum` 改动 router wiring，可能影响 middleware 顺序。**
   保留 `router(state) -> Router` 的外部签名，并用现有安全、CORS、rate-limit 测试覆盖行为。

2. **核心类型派生 `ToSchema` 可能引入文档依赖到 core 层。**
   先评估每个类型的耦合；对不适合的类型使用 API docs wrapper，避免为了文档污染核心抽象。

3. **SSE schema 难以完整表达 streaming behavior。**
   OpenAPI 中明确标记 `text/event-stream`，body 描述为 `JobStreamEvent` stream，而不是承诺普通
   JSON array。

4. **Swagger UI 可访问但业务 API 仍可能要求 token。**
   测试需要覆盖 OpenAPI 中的 `BearerAuth` 声明，并保留现有 token auth 行为测试，避免文档端点
   的可访问性被误解为业务 API 放宽认证。

5. **OpenAPI spec 与实际 API 发生漂移。**
   通过 `utoipa_axum` 的 route registration 和 paths 回归测试降低漂移风险。

## Implementation Boundary

本设计完成后，下一步应先写实现计划，再开始改代码。实现计划需要拆成：

1. 依赖和 docs module scaffold。
2. OpenAPI router wiring。
3. DTO/schema/path annotations。
4. OpenAPI/Swagger UI regression tests。
5. Runtime documentation updates。
6. Full verification commands.

所有实现工作继续在 `.worktrees/utoipa-api-docs` 中进行。
