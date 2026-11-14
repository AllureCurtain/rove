# Pilot notes — defects, friction, surprises

Every item here is a real pilot finding, not an annoyance.

## D1 — `deepseek-chat` no longer exists on DeepSeek's live catalog
- `GET https://api.deepseek.com/v1/models` (2026-11-14) returns only
  `deepseek-flash` and `deepseek-v4-pro`. `deepseek-chat` is gone.
- Onboarding rejected the typed model with HTTP 400
  `provider_model_unavailable: "Selected model was not present in Provider inventory"`.
- Fixed by using `deepseek-v4-pro`. Any docs/runbooks/briefs naming `deepseek-chat`
  are now stale.

## D2 — Provider onboarding UX trap (key consumed, profile not saved)
- Flow: enter label/type/api-base/default-model + key → "Verify and save".
- When the typed default model isn't in the fetched inventory, the call fails —
  but the entered key field is cleared ("consumed") while the profile is NOT saved.
- Result: subsequent "Test connection" / "List models" fail with
  "check the credential source" because they only use a *saved* credential or an env var.
- Chicken-and-egg: you cannot pick a valid model from "List models" until a credential
  is saved, and you cannot save until the model validates. The only escape is typing a
  model name that happens to be in the live catalog. Friction + a silent key wipe.

## D3 — `runtime.execution` in project `.rove/config.toml` is ignored for product sessions
- Created `/Users/yao/dev/projects/rove/.rove/config.toml` with
  `runtime.execution.max_model_turns_per_step = 16`, `max_model_turns = 60`.
- Granted `project_configuration` + `provider_credentials` trust (durable, state=trusted).
- Restarted `rove-api`, opened a fresh session → run STILL exhausted at
  `max_model_turns_per_step=4`.
- Note: the same `[runtime.execution]` block placed in `$ROVE_CONFIG_ROOT/config.toml`
  makes the API fail to start (`unknown field 'runtime'`) — that file is the product/
  provider config, not AppConfig. So the only documented override location
  (project `.rove/config.toml`) is not applied to the API session engine.
- Conclusion: the per-step model-turn budget is effectively hardwired for web/API runs.

## D4 — Default `max_model_turns_per_step = 4` too small for tool-using tasks
- Located at `apps/bootstrap/src/config.rs` (~line 3372 test default) / ExecutionConfig
  default in `crates/foundation/src/execution.rs`.
- Each "Thinking → parallel tool batch" cycle consumes a model turn. DeepSeek-v4-pro
  issues up to 5 tool calls per turn, so 4 turns = retrieval only, no synthesis turn.
- Symptom: run streams lots of real tool calls then dies with
  `Execution reached its limit … BudgetExhausted` and shows a derived summary instead of
  the model's answer.

## Positive observations
- UI onboarding → keyring storage works; key never appears in config.toml, logs, or export.
- `provider_credentials` trust prompt surfaced correctly and Authorize worked inline.
- Real streaming + real tool execution against the repo; token/usage meter is accurate
  and visible ("N actions · NNk tokens", context chip).
- Session export endpoint (`POST /product/sessions/{id}/export?format=json`) returns a
  redacted 153 KB evidence bundle — good for pilot/audit.
- No runtime crash, no auth failure, no empty/garbage output across 3 runs.
