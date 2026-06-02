# Provider Gates And Stress Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend rove's real-provider release gates so Anthropic and Ollama have the same API/Web evidence path as OpenAI-compatible providers, and make stress/long-running provider tests repeatable, classified, and auditable.

**Architecture:** Keep the runtime provider adapters unchanged unless a gate exposes a real bug. Add a provider-neutral gate runner layer around the existing API/Web/runtime surfaces, with protocol-specific inventory and smoke commands hidden behind small PowerShell functions. Stress testing should reuse the same isolated API workspace and write structured artifacts for sequential, concurrent, restart-recovery, and long-run evidence.

**Tech Stack:** PowerShell runner scripts, Rust API/runtime tests, Next.js workbench Playwright automation, JSON artifacts, GitHub Actions-compatible local commands.

---

## 阅读导引

这份计划只覆盖本轮明确要做的两块：第二点“门禁扩展”和第三点“压力长跑测试”。其他产品方向、功能体验、发布叙事先不进入本计划。

阅读时先看下面三段：

- `为什么这么做 / Why We Are Doing This`：说明现在为什么不能只靠 OpenAI-compatible 门禁，以及压力测试为什么需要更长、更可审计。
- `整体思路 / Overall Approach`：说明实现要分成协议门禁扩展和压力子门禁两层。
- `验收标准 / Acceptance Standards`：说明什么结果才算这轮实现真正可交付。

后面的 Task 1 到 Task 12 是可执行任务拆解，按顺序做即可。每个任务都包含要改的文件、建议代码、验证命令和预期结果。

## 为什么这么做 / Why We Are Doing This

The previous work made provider selection generic at product level: `POST /jobs` and the Web workbench can route a single run through OpenAI-compatible, Anthropic, Ollama, or fake providers without sending raw keys from the browser. That is enough for manual use, but not enough for release confidence.

Right now the automated full provider gate is strongest for OpenAI-compatible targets because official APIs, relay APIs, and gateway APIs share a common `/v1` surface. Anthropic and Ollama have native adapters and opt-in smoke tests, but they do not yet have the same one-command evidence package: model inventory, API run, Web run, tool-use record, artifacts, and classification.

Stress testing is also too shallow for release readiness. The current `-RunStress` path runs a small sequential/concurrent batch, but it does not prove restart recovery, long-running behavior, provider error classification over time, or artifact completeness under extended load. We need a gate that can run longer without becoming opaque.

The target is not to make real providers deterministic. The target is to make provider variability visible, classified, and separate from rove runtime defects.

## 整体思路 / Overall Approach

Build this in two layers.

First, turn `scripts/provider-integration.ps1` from an OpenAI-compatible-only runner into a protocol-aware runner. It should keep the existing command shape, but dispatch internally by `-Provider`:

- `openai-compatible`: current behavior, including `/models`, `OPENAI_API_KEY`, provider smoke test, API jobs, Web jobs, optional stress, optional external MCP.
- `anthropic`: use `ANTHROPIC_API_KEY` by default, inventory through Anthropic `/v1/models`, smoke through `tests/provider_smoke.rs`, API jobs through per-run provider profiles, Web jobs through the workbench provider selector.
- `ollama`: no key required, inventory through `/api/tags`, smoke through `tests/provider_smoke.rs`, API jobs through per-run provider profiles, Web jobs through the workbench provider selector.

Second, split stress into explicit sub-gates:

- short sequential stress;
- short concurrent stress;
- restart recovery after completed provider jobs;
- longer sequential soak;
- optional external MCP after provider gates pass.

Each sub-gate should produce JSON artifacts with counts, run ids, terminal statuses, classification, and links to state/report files. The runner should fail when rove loses run records, corrupts state, panics, or misclassifies errors. Provider 401/403/429/network/model-tool-use failures should be classified distinctly and written to evidence.

## Non-Goals

- Do not add a new model provider adapter.
- Do not make provider outputs deterministic.
- Do not introduce hosted auth, SaaS concepts, billing, or distributed rate limiting.
- Do not run long stress by default in normal CI.
- Do not commit real provider keys, bearer tokens, generated state, screenshots, SQLite files, or logs.

## Current State

Important files:

- `scripts/provider-integration.ps1`: current full runner for OpenAI-compatible providers; already has `-RunStress` and `-RunExternalMcp` switches.
- `tests/provider_smoke.rs`: opt-in smoke tests for OpenAI-compatible, Anthropic, and Ollama.
- `src/interfaces/api/mod.rs`: accepts per-run provider profiles and `POST /providers/test`.
- `web-ui/components/rove-workbench.tsx`: provider selector for runtime default, OpenAI-compatible, Anthropic, Ollama, and fake.
- `web-ui/tests/e2e/workbench.spec.ts`: mocked browser coverage for provider profile payloads.
- `docs/runtime/provider-smoke.md`, `docs/runtime/integration-testing.md`, `docs/runtime/full-integration-runbook.md`, `docs/runtime/release-readiness.md`: user-facing gate docs.

Current known gap:

- `scripts/provider-integration.ps1` rejects non-OpenAI-compatible providers in `Set-ProviderEnvironment`.
- The Web smoke script inside `scripts/provider-integration.ps1` fills only `Task`, `Model`, and `Steps`; it does not select Anthropic/Ollama provider profiles.
- Stress does not restart the API and verify `/runs` after restart.
- Stress writes summary counts, but not enough classification and per-run report evidence for a long run.

## 验收标准 / Acceptance Standards

This work is accepted only when all of the following are true.

### Functional Acceptance

1. `scripts/provider-integration.ps1 -Provider openai-compatible ...` keeps the current behavior and still passes against a real OpenAI-compatible endpoint.
2. `scripts/provider-integration.ps1 -Provider anthropic ...` can run model inventory, provider smoke, API provider jobs, Web provider jobs, and evidence summary when `ANTHROPIC_API_KEY` and an accessible model are present.
3. `scripts/provider-integration.ps1 -Provider ollama ...` can run model inventory, provider smoke, API provider jobs, Web provider jobs, and evidence summary against a local Ollama server with no API key.
4. `-RunStress` records sequential and concurrent provider job evidence for every supported provider mode.
5. `-RunStress -RunRestartRecovery` or equivalent restart option proves completed provider runs remain visible after API restart.
6. Long stress can be configured without editing the script, using counts and timeouts from flags or environment variables.
7. `-RunExternalMcp` remains available and can run after provider gates without using the real provider for the deterministic MCP fixture unless explicitly requested.

### Evidence Acceptance

For every provider run, the artifact directory must contain:

- `environment.redacted.json`;
- provider inventory artifact;
- selected model id;
- provider smoke result;
- API plain/tool created/state/report artifacts;
- Web screenshot and report artifacts when Web smoke is enabled;
- `evidence-summary.json` with gate status for every requested gate;
- no raw API key or bearer token.

For stress runs, the artifact directory must contain:

- `stress-summary.json`;
- per-job state artifacts;
- per-job report artifacts, at least for failed jobs and a representative sample of successful jobs;
- `stress-runs-before-restart.json` and `stress-runs-after-restart.json` when restart recovery is enabled;
- API stdout/stderr logs for the stress process before and after restart;
- provider error classifications if any job fails.

### Quality Acceptance

1. `cargo fmt --all --check` passes.
2. `cargo clippy --all-targets -- -D warnings` passes.
3. `cargo test` passes.
4. `cd web-ui; pnpm test; pnpm typecheck; pnpm build` passes.
5. Focused Playwright coverage for provider selector payloads passes.
6. No generated artifacts, runtime state, SQLite files, screenshots, traces, or secrets appear in `git status --short`.
7. `git diff --check` passes.

## File Map

- Modify: `scripts/provider-integration.ps1`
  - Add provider protocol normalization.
  - Add Anthropic and Ollama inventory.
  - Add provider-specific smoke command dispatch.
  - Add API/Web job creation through per-run provider profiles.
  - Add restart recovery and long-stress artifacts.

- Modify: `.env.integration.example`
  - Add generic provider gate variables for Anthropic/Ollama defaults.
  - Add stress timeout/count variables.
  - Add restart-recovery and long-soak switches.

- Modify: `tests/code_hygiene.rs`
  - Assert the runner documents and exposes provider protocol dispatch.
  - Assert restart recovery and stress artifact names exist.

- Modify: `web-ui/tests/e2e/workbench.spec.ts`
  - Keep existing provider selector payload coverage.
  - Add only if the runner's browser script is extracted into reusable app behavior.

- Modify: `docs/runtime/provider-smoke.md`
  - Document provider-specific full gate commands.

- Modify: `docs/runtime/integration-testing.md`
  - Document the expanded runner and stress profiles.

- Modify: `docs/runtime/release-readiness.md`
  - Update release gate checklist with provider matrix and long-stress expectations.

- Modify: `docs/runtime/full-integration-runbook.md`
  - Keep as operator handoff, with commands for OpenAI-compatible, Anthropic, Ollama, stress, restart, and external MCP.

## Provider Protocol Matrix

| Provider | Key Env Default | Inventory | Smoke Test | API/Web Job Profile |
|---|---|---|---|---|
| `openai-compatible` | `OPENAI_API_KEY` | `GET <api_base>/models`, bearer auth | `openai_compatible_real_provider_smoke_when_enabled` | `name=openai-compatible`, `api_base`, `api_key_env` |
| `anthropic` | `ANTHROPIC_API_KEY` | `GET <api_base>/v1/models`, `x-api-key`, `anthropic-version` | `anthropic_real_provider_smoke_when_enabled` | `name=anthropic`, `api_base`, `api_key_env` |
| `ollama` | none | `GET <api_base>/api/tags` | `ollama_real_provider_smoke_when_enabled` | `name=ollama`, `api_base`, no `api_key_env` |

## Task 1: Add Runner Protocol Dispatch Tests

**Files:**
- Modify: `tests/code_hygiene.rs`
- No production script changes in this task.

- [ ] **Step 1: Add a failing hygiene test for provider dispatch**

Append this test to `tests/code_hygiene.rs`:

```rust
#[test]
fn provider_integration_runner_supports_native_provider_protocols() {
    let script = std::fs::read_to_string("scripts/provider-integration.ps1")
        .expect("scripts/provider-integration.ps1 should exist");
    let provider_docs = std::fs::read_to_string("docs/runtime/provider-smoke.md").unwrap();
    let readiness = std::fs::read_to_string("docs/runtime/release-readiness.md").unwrap();

    assert!(script.contains("function Normalize-ProviderName"));
    assert!(script.contains("function Invoke-AnthropicModelInventory"));
    assert!(script.contains("function Invoke-OllamaModelInventory"));
    assert!(script.contains("anthropic_real_provider_smoke_when_enabled"));
    assert!(script.contains("ollama_real_provider_smoke_when_enabled"));
    assert!(script.contains("provider = @{"));
    assert!(script.contains("name = $Provider"));
    assert!(script.contains("api_key_env = $ApiKeyEnv"));
    assert!(script.contains("ROVE_PROVIDER_SMOKE_ANTHROPIC"));
    assert!(script.contains("ROVE_PROVIDER_SMOKE_OLLAMA"));
    assert!(!script.contains("currently automates API/Web gates for openai-compatible providers"));

    assert!(provider_docs.contains("-Provider anthropic"));
    assert!(provider_docs.contains("-Provider ollama"));
    assert!(readiness.contains("Provider Gate Matrix"));
}
```

- [ ] **Step 2: Run the failing test**

Run:

```powershell
cargo test --test code_hygiene provider_integration_runner_supports_native_provider_protocols -- --exact
```

Expected: FAIL because `Normalize-ProviderName`, Anthropic/Ollama inventory functions, and docs do not exist yet.

- [ ] **Step 3: Commit the failing test only if working in a TDD branch**

Run:

```powershell
git add tests/code_hygiene.rs
git commit -m "test: define provider integration protocol dispatch"
```

Expected: commit succeeds. If your team's convention avoids red commits, skip this commit and keep the test staged until Task 2 turns it green.

## Task 2: Normalize Provider Inputs In The Runner

**Files:**
- Modify: `scripts/provider-integration.ps1`

- [ ] **Step 1: Add provider normalization helpers near existing utility functions**

Add these functions after `Test-CommandAvailable`:

```powershell
function Normalize-ProviderName([string]$Name) {
    $normalized = $Name.Trim().ToLowerInvariant()
    switch ($normalized) {
        "openai" { return "openai-compatible" }
        "openai-compatible" { return "openai-compatible" }
        "anthropic" { return "anthropic" }
        "ollama" { return "ollama" }
        default {
            throw "Unsupported provider '$Name'. Expected openai-compatible, anthropic, or ollama."
        }
    }
}

function Provider-RequiresKey([string]$Name) {
    $normalized = Normalize-ProviderName $Name
    return $normalized -in @("openai-compatible", "anthropic")
}

function Default-KeyEnvForProvider([string]$Name) {
    $normalized = Normalize-ProviderName $Name
    if ($normalized -eq "anthropic") {
        return "ANTHROPIC_API_KEY"
    }
    return "OPENAI_API_KEY"
}

function Default-ApiBaseForProvider([string]$Name, [string]$CurrentBase) {
    $normalized = Normalize-ProviderName $Name
    if ($CurrentBase) {
        return $CurrentBase.TrimEnd("/")
    }
    switch ($normalized) {
        "anthropic" { return "https://api.anthropic.com" }
        "ollama" { return "http://localhost:11434" }
        default { return "https://api.openai.com/v1" }
    }
}
```

- [ ] **Step 2: Normalize `$Provider`, `$ApiKeyEnv`, and `$ApiBase` after loading `.env.integration`**

Replace the post-import setup block:

```powershell
Set-ProviderEnvironment
```

with:

```powershell
$Provider = Normalize-ProviderName $Provider
if (-not $ApiKeyEnv) {
    $ApiKeyEnv = Default-KeyEnvForProvider $Provider
}
$ApiBase = Default-ApiBaseForProvider $Provider $ApiBase
Set-ProviderEnvironment
```

- [ ] **Step 3: Update `Get-ApiKeyValue` for keyless providers**

Replace the start of `Get-ApiKeyValue` with:

```powershell
function Get-ApiKeyValue {
    if (-not (Provider-RequiresKey $Provider)) {
        return ""
    }
    $value = [Environment]::GetEnvironmentVariable($ApiKeyEnv, "Process")
    if (-not $value -and $Provider -eq "openai-compatible" -and $ApiKeyEnv -ne "OPENAI_API_KEY") {
        $value = [Environment]::GetEnvironmentVariable("OPENAI_API_KEY", "Process")
    }
    if (-not $value) {
        throw "Provider API key is not set. Expected environment variable '$ApiKeyEnv'."
    }
    return $value
}
```

Do not keep the old fallback to `OPENAI_API_KEY` for Anthropic.

- [ ] **Step 4: Update `Set-ProviderEnvironment`**

Replace the function body with:

```powershell
function Set-ProviderEnvironment {
    $key = Get-ApiKeyValue
    $env:ROVE_PROVIDER = $Provider
    $env:ROVE_MODEL = $Model

    if ($Provider -eq "openai-compatible") {
        $env:OPENAI_API_KEY = $key
        $env:OPENAI_API_BASE = $ApiBase
        return
    }

    if ($Provider -eq "anthropic") {
        $env:ANTHROPIC_API_KEY = $key
        $env:ROVE_PROVIDER_API_BASE = $ApiBase
        return
    }

    if ($Provider -eq "ollama") {
        $env:ROVE_PROVIDER_API_BASE = $ApiBase
        return
    }
}
```

- [ ] **Step 5: Run the targeted hygiene test**

Run:

```powershell
cargo test --test code_hygiene provider_integration_runner_supports_native_provider_protocols -- --exact
```

Expected: still FAIL because inventory and docs are not complete. This is fine.

## Task 3: Add Provider-Specific Inventory

**Files:**
- Modify: `scripts/provider-integration.ps1`
- Modify: `tests/code_hygiene.rs` if the exact function names differ

- [ ] **Step 1: Replace `Get-DefaultModelsEndpoint` with protocol-aware endpoint helpers**

Use:

```powershell
function Get-DefaultModelsEndpoint {
    if ($ModelsEndpoint) {
        return $ModelsEndpoint
    }
    switch ($Provider) {
        "anthropic" { return ($ApiBase.TrimEnd("/") + "/v1/models") }
        "ollama" { return ($ApiBase.TrimEnd("/") + "/api/tags") }
        default { return ($ApiBase.TrimEnd("/") + "/models") }
    }
}
```

- [ ] **Step 2: Split inventory functions**

Add these functions before `Invoke-ModelInventory`:

```powershell
function Invoke-OpenAiCompatibleModelInventory {
    $endpoint = Get-DefaultModelsEndpoint
    $headers = @{ Authorization = "Bearer $(Get-ApiKeyValue)" }
    $models = Invoke-RestMethod -Uri $endpoint -Headers $headers -TimeoutSec 30
    $items = @($models.data)
    $visible = $items | Where-Object { $_.id } | Sort-Object id
    $visible | Select-Object id, owned_by | ConvertTo-Json -Depth 20 |
        Set-Content -LiteralPath (Join-Path $ArtifactsDir "provider-models.json")
    return $visible.id
}

function Invoke-AnthropicModelInventory {
    $endpoint = Get-DefaultModelsEndpoint
    $headers = @{
        "x-api-key" = Get-ApiKeyValue
        "anthropic-version" = "2023-06-01"
    }
    $models = Invoke-RestMethod -Uri $endpoint -Headers $headers -TimeoutSec 30
    $items = @($models.data)
    $visible = $items | Where-Object { $_.id } | Sort-Object id
    $visible | Select-Object id, display_name, created_at | ConvertTo-Json -Depth 20 |
        Set-Content -LiteralPath (Join-Path $ArtifactsDir "provider-models.json")
    return $visible.id
}

function Invoke-OllamaModelInventory {
    $endpoint = Get-DefaultModelsEndpoint
    $models = Invoke-RestMethod -Uri $endpoint -TimeoutSec 30
    $items = @($models.models)
    $visible = $items | Where-Object { $_.name } | Sort-Object name
    $visible | Select-Object name, model, modified_at, size | ConvertTo-Json -Depth 20 |
        Set-Content -LiteralPath (Join-Path $ArtifactsDir "provider-models.json")
    return $visible.name
}
```

- [ ] **Step 3: Replace `Invoke-ModelInventory`**

Use:

```powershell
function Invoke-ModelInventory {
    if ($SkipModelInventory) {
        return "skipped"
    }

    $modelIds = switch ($Provider) {
        "anthropic" { Invoke-AnthropicModelInventory }
        "ollama" { Invoke-OllamaModelInventory }
        default { Invoke-OpenAiCompatibleModelInventory }
    }

    $Model | Set-Content -LiteralPath (Join-Path $ArtifactsDir "selected-provider-model.txt")

    if (-not ($modelIds | Where-Object { $_ -eq $Model } | Select-Object -First 1)) {
        throw "Model '$Model' was not present in provider model inventory from $(Get-DefaultModelsEndpoint)."
    }
    return "pass"
}
```

- [ ] **Step 4: Run focused checks**

Run:

```powershell
cargo test --test code_hygiene provider_integration_runner_supports_native_provider_protocols -- --exact
```

Expected: may still FAIL until smoke/docs tasks are complete.

## Task 4: Dispatch Provider Smoke By Provider

**Files:**
- Modify: `scripts/provider-integration.ps1`

- [ ] **Step 1: Replace `Invoke-ProviderSmoke` command selection**

Inside `Invoke-ProviderSmoke`, replace the fixed OpenAI-compatible env/test setup with:

```powershell
function Invoke-ProviderSmoke {
    if ($SkipProviderSmoke) {
        return "skipped"
    }

    $testName = ""
    switch ($Provider) {
        "openai-compatible" {
            $env:ROVE_PROVIDER_SMOKE_OPENAI = "1"
            $env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL = $Model
            $testName = "openai_compatible_real_provider_smoke_when_enabled"
        }
        "anthropic" {
            $env:ROVE_PROVIDER_SMOKE_ANTHROPIC = "1"
            $env:ROVE_PROVIDER_SMOKE_ANTHROPIC_MODEL = $Model
            $testName = "anthropic_real_provider_smoke_when_enabled"
        }
        "ollama" {
            $env:ROVE_PROVIDER_SMOKE_OLLAMA = "1"
            $env:ROVE_PROVIDER_SMOKE_OLLAMA_MODEL = $Model
            $testName = "ollama_real_provider_smoke_when_enabled"
        }
    }

    $log = Join-Path $ArtifactsDir "provider-smoke.log"
    $previousErrorActionPreference = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & cargo test --test provider_smoke $testName -- --exact --nocapture 2>&1
        $exit = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    $output | Set-Content -LiteralPath $log
    $classification = Classify-ProviderOutput -ExitCode $exit -Text ($output -join "`n")
    @{
        provider = $Provider
        model = $Model
        test = $testName
        exit_code = $exit
        classification = $classification
        log = $log
    } | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "provider-smoke-result.json")
    if ($exit -ne 0) {
        throw "provider smoke failed with classification '$classification'. See $log"
    }
    return $classification
}
```

- [ ] **Step 2: Ensure env variables do not leak between smoke modes**

Before setting provider-specific smoke env vars in `Invoke-ProviderSmoke`, add:

```powershell
$env:ROVE_PROVIDER_SMOKE_OPENAI = "0"
$env:ROVE_PROVIDER_SMOKE_ANTHROPIC = "0"
$env:ROVE_PROVIDER_SMOKE_OLLAMA = "0"
```

- [ ] **Step 3: Run skipped smoke tests locally**

Run:

```powershell
cargo test --test provider_smoke
```

Expected: PASS with all real-provider gates skipped unless env flags are set.

## Task 5: Make API Smoke Use Per-Run Provider Profiles

**Files:**
- Modify: `scripts/provider-integration.ps1`

- [ ] **Step 1: Add provider profile body helper**

Add this helper before `Invoke-ApiSmoke`:

```powershell
function New-ProviderProfileBody {
    $profile = @{
        name = $Provider
        api_base = $ApiBase
    }
    if (Provider-RequiresKey $Provider) {
        $profile.api_key_env = $ApiKeyEnv
    }
    return $profile
}
```

- [ ] **Step 2: Include provider profile in API job bodies**

In `Invoke-ApiSmoke`, change both `$plain` and `$tool` job bodies from:

```powershell
@{
    message = "..."
    model = $Model
    approval = "auto"
    max_steps = 4
}
```

to:

```powershell
@{
    message = "..."
    model = $Model
    approval = "auto"
    max_steps = 4
    provider = New-ProviderProfileBody
}
```

- [ ] **Step 3: Verify artifact output includes provider profile**

After a real or local fake dry run, inspect:

```powershell
Get-Content <artifacts>\provider-full-plain.created.json
Get-Content <artifacts>\provider-full-tool.created.json
```

Expected: created job responses do not echo secrets. Provider profile appears only in request logs if you add separate redacted request artifacts. Do not write raw key values.

## Task 6: Make Web Smoke Select Provider Profiles

**Files:**
- Modify: `scripts/provider-integration.ps1`

- [ ] **Step 1: Pass provider settings into the Node script**

Before `$nodeScript = @'`, set:

```powershell
$env:ROVE_WEB_PROVIDER = $Provider
$env:ROVE_WEB_PROVIDER_API_BASE = $ApiBase
$env:ROVE_WEB_PROVIDER_KEY_ENV = $ApiKeyEnv
```

- [ ] **Step 2: Update the Node Playwright script constants**

Replace the script's constant section:

```javascript
const model = process.env.ROVE_MODEL;
```

with:

```javascript
const model = process.env.ROVE_MODEL;
const provider = process.env.ROVE_WEB_PROVIDER;
const providerApiBase = process.env.ROVE_WEB_PROVIDER_API_BASE;
const providerKeyEnv = process.env.ROVE_WEB_PROVIDER_KEY_ENV;
```

- [ ] **Step 3: Select provider in the Web UI**

After `await page.goto(...)`, insert:

```javascript
if (provider && provider !== 'default') {
  await page.getByLabel('Provider').selectOption(provider);
  await page.getByLabel('API base').fill(providerApiBase);
  const keyEnv = page.getByLabel('Key env');
  if (await keyEnv.count()) {
    await keyEnv.fill(providerKeyEnv);
  }
}
```

- [ ] **Step 4: Include provider details in `web-provider-result.json`**

Add these fields to the `result` object:

```javascript
provider,
providerApiBase,
providerKeyEnv: providerKeyEnv || '',
```

- [ ] **Step 5: Run mocked E2E after script change**

Run:

```powershell
cd web-ui
$env:ROVE_WEB_PORT = "13160"
$env:PLAYWRIGHT_BASE_URL = "http://127.0.0.1:13160"
pnpm test:e2e -- tests/e2e/workbench.spec.ts
cd ..
```

Expected: PASS.

## Task 7: Add Restart Recovery Stress Gate

**Files:**
- Modify: `scripts/provider-integration.ps1`
- Modify: `.env.integration.example`
- Modify: `tests/code_hygiene.rs`

- [ ] **Step 1: Add parameters**

Add these parameters after `-RunStress`:

```powershell
[switch]$RunRestartRecovery,
[int]$StressJobTimeoutSeconds = $(if ($env:ROVE_PROVIDER_INTEGRATION_STRESS_JOB_TIMEOUT_SECONDS) { [int]$env:ROVE_PROVIDER_INTEGRATION_STRESS_JOB_TIMEOUT_SECONDS } else { 180 }),
[int]$RestartRecoveryTimeoutSeconds = $(if ($env:ROVE_PROVIDER_INTEGRATION_RESTART_TIMEOUT_SECONDS) { [int]$env:ROVE_PROVIDER_INTEGRATION_RESTART_TIMEOUT_SECONDS } else { 90 }),
```

- [ ] **Step 2: Use timeout parameter in stress waits**

Replace hard-coded stress wait timeouts:

```powershell
Wait-JobTerminal -JobId $job.job_id -Name "stress-sequential-$i" -TimeoutSeconds 180
Wait-JobTerminal -JobId $jobs[$i].job_id -Name "stress-concurrent-$($i + 1)" -TimeoutSeconds 180
```

with:

```powershell
Wait-JobTerminal -JobId $job.job_id -Name "stress-sequential-$i" -TimeoutSeconds $StressJobTimeoutSeconds
Wait-JobTerminal -JobId $jobs[$i].job_id -Name "stress-concurrent-$($i + 1)" -TimeoutSeconds $StressJobTimeoutSeconds
```

- [ ] **Step 3: Add restart helper**

Add:

```powershell
function Invoke-RestartRecoveryGate([array]$CreatedJobs, [string]$StressWorkspace) {
    if (-not $RunRestartRecovery) {
        return "skipped"
    }
    $before = Invoke-Json -Method Get -Uri "$ApiBaseLocal/runs?limit=100"
    $before | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "stress-runs-before-restart.json")

    if ($script:StressApiProcess) {
        Stop-ProcessTree $script:StressApiProcess
        $script:StressApiProcess = $null
    }

    $script:StressApiProcess = Start-BackgroundCommand -Command "cargo" -Arguments @("run", "--bin", "rove-api", "--", "--addr", $ApiAddr, "-C", $StressWorkspace) -WorkingDirectory $RepoRoot -StdoutLog (Join-Path $ArtifactsDir "stress-api-restart.out.log") -StderrLog (Join-Path $ArtifactsDir "stress-api-restart.err.log")
    Wait-HttpOk -Uri "$ApiBaseLocal/runs?limit=1" -TimeoutSeconds $RestartRecoveryTimeoutSeconds -Name "restarted stress rove-api"

    $after = Invoke-Json -Method Get -Uri "$ApiBaseLocal/runs?limit=100"
    $after | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "stress-runs-after-restart.json")

    $afterIds = @($after.runs | ForEach-Object { [string]$_.run_id })
    foreach ($job in $CreatedJobs) {
        if (-not ($afterIds -contains [string]$job.run_id)) {
            throw "restart recovery lost run_id $($job.run_id)"
        }
    }
    return "pass"
}
```

- [ ] **Step 4: Store API process in script scope in `Invoke-StressGate`**

In `Invoke-StressGate`, replace local `$apiProcess` usage with `$script:StressApiProcess` so the restart helper can stop and restart it. In the `finally`, stop `$script:StressApiProcess` if it exists.

- [ ] **Step 5: Call restart helper before writing stress summary**

After concurrent jobs finish and before fetching final runs, add:

```powershell
$restartStatus = Invoke-RestartRecoveryGate -CreatedJobs $created -StressWorkspace $stressWorkspace
```

Then include this in `stress-summary.json`:

```powershell
restart_recovery = $restartStatus
```

- [ ] **Step 6: Update `.env.integration.example`**

Add:

```dotenv
ROVE_PROVIDER_INTEGRATION_STRESS_JOB_TIMEOUT_SECONDS=180
ROVE_PROVIDER_INTEGRATION_RESTART_TIMEOUT_SECONDS=90
```

- [ ] **Step 7: Update hygiene test**

In `provider_integration_runner_is_generic_and_documented`, add assertions:

```rust
assert!(script.contains("[switch]$RunRestartRecovery"));
assert!(script.contains("Invoke-RestartRecoveryGate"));
assert!(script.contains("stress-runs-before-restart.json"));
assert!(script.contains("stress-runs-after-restart.json"));
```

- [ ] **Step 8: Run focused tests**

Run:

```powershell
cargo test --test code_hygiene
```

Expected: PASS.

## Task 8: Add Long Soak Stress Mode

**Files:**
- Modify: `scripts/provider-integration.ps1`
- Modify: `.env.integration.example`

- [ ] **Step 1: Add parameters**

Add:

```powershell
[switch]$RunLongSoak,
[int]$LongSoakCount = $(if ($env:ROVE_PROVIDER_INTEGRATION_LONG_SOAK_COUNT) { [int]$env:ROVE_PROVIDER_INTEGRATION_LONG_SOAK_COUNT } else { 20 }),
[int]$LongSoakDelayMs = $(if ($env:ROVE_PROVIDER_INTEGRATION_LONG_SOAK_DELAY_MS) { [int]$env:ROVE_PROVIDER_INTEGRATION_LONG_SOAK_DELAY_MS } else { 500 }),
```

Do not enable `RunLongSoak` by default.

- [ ] **Step 2: Add long soak helper**

Add:

```powershell
function Invoke-LongSoakGate {
    if (-not $RunLongSoak) {
        return "skipped"
    }
    $jobs = @()
    for ($i = 1; $i -le $LongSoakCount; $i++) {
        $job = Invoke-Json -Method Post -Uri "$ApiBaseLocal/jobs" -Body @{
            message = "Reply with exactly: rove provider long soak ok $i"
            model = $Model
            approval = "auto"
            max_steps = 4
            provider = New-ProviderProfileBody
        }
        $state = Wait-JobTerminal -JobId $job.job_id -Name "long-soak-$i" -TimeoutSeconds $StressJobTimeoutSeconds
        $report = Invoke-Json -Method Get -Uri "$ApiBaseLocal/runs/$($job.run_id)/report"
        $report | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "long-soak-$i.report.json")
        $jobs += @{
            index = $i
            job_id = [string]$job.job_id
            run_id = [string]$job.run_id
            status = [string]$state.status
            report_status = [string]$report.status
            output = [string]$report.output
        }
        if ($state.status -ne "done" -or $report.status -ne "success") {
            $classification = Classify-ProviderOutput -ExitCode 1 -Text ([string]$report.output)
            @{
                failed_index = $i
                classification = $classification
                jobs = $jobs
            } | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "long-soak-summary.json")
            throw "long soak job $i failed with classification '$classification'"
        }
        Start-Sleep -Milliseconds $LongSoakDelayMs
    }
    @{
        count = $LongSoakCount
        delay_ms = $LongSoakDelayMs
        jobs = $jobs
    } | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "long-soak-summary.json")
    return "pass"
}
```

- [ ] **Step 3: Call long soak after restart recovery**

Inside `Invoke-StressGate`, after restart recovery and before final `stress-summary.json`, call:

```powershell
$longSoakStatus = Invoke-LongSoakGate
```

Include:

```powershell
long_soak = $longSoakStatus
```

- [ ] **Step 4: Update `.env.integration.example`**

Add:

```dotenv
ROVE_PROVIDER_INTEGRATION_LONG_SOAK_COUNT=20
ROVE_PROVIDER_INTEGRATION_LONG_SOAK_DELAY_MS=500
```

- [ ] **Step 5: Run hygiene tests**

Run:

```powershell
cargo test --test code_hygiene
```

Expected: PASS.

## Task 9: Improve Stress Error Classification

**Files:**
- Modify: `scripts/provider-integration.ps1`

- [ ] **Step 1: Add report classifier**

Add:

```powershell
function Classify-RunReport([object]$Report, [object]$State) {
    $text = (($Report.output, ($State | ConvertTo-Json -Depth 20 -Compress)) -join "`n")
    if ($State.status -eq "done" -and $Report.status -eq "success") {
        return "pass"
    }
    return Classify-ProviderOutput -ExitCode 1 -Text $text
}
```

- [ ] **Step 2: Use classifier for sequential and concurrent stress jobs**

After each stress job reaches terminal state, fetch its report:

```powershell
$report = Invoke-Json -Method Get -Uri "$ApiBaseLocal/runs/$($job.run_id)/report"
$report | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "stress-sequential-$i.report.json")
$classification = Classify-RunReport -Report $report -State $state
```

Add `classification` and `report_status` to each `$created += @{ ... }` record.

- [ ] **Step 3: Fail only on unaccepted classifications**

Replace:

```powershell
if ($state.status -ne "done") {
    throw "sequential stress job $i ended with status $($state.status)"
}
```

with:

```powershell
if ($classification -ne "pass") {
    throw "sequential stress job $i ended with classification '$classification'"
}
```

Do the same for concurrent jobs.

- [ ] **Step 4: Run a small local fake stress if possible**

Because the runner is provider-focused, use Ollama only if a local server is available. Otherwise run code hygiene and skip real network:

```powershell
cargo test --test code_hygiene
```

Expected: PASS.

## Task 10: Update Documentation

**Files:**
- Modify: `docs/runtime/provider-smoke.md`
- Modify: `docs/runtime/integration-testing.md`
- Modify: `docs/runtime/full-integration-runbook.md`
- Modify: `docs/runtime/release-readiness.md`

- [ ] **Step 1: Add provider full-gate examples**

In `docs/runtime/provider-smoke.md`, add commands:

```powershell
# OpenAI-compatible official API, relay, or gateway
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai-compatible `
  -ApiBase "https://<provider-or-gateway>/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "<model-id>"

# Anthropic
$env:ANTHROPIC_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider anthropic `
  -ApiBase "https://api.anthropic.com" `
  -ApiKeyEnv ANTHROPIC_API_KEY `
  -Model "claude-3-5-haiku-latest"

# Ollama
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider ollama `
  -ApiBase "http://localhost:11434" `
  -Model "llama3.2"
```

- [ ] **Step 2: Add stress examples**

Add:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai-compatible `
  -ApiBase "https://<provider-or-gateway>/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "<model-id>" `
  -RunStress `
  -RunRestartRecovery `
  -StressSequentialCount 20 `
  -StressConcurrentCount 5
```

Add long soak example:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai-compatible `
  -ApiBase "https://<provider-or-gateway>/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "<model-id>" `
  -RunStress `
  -RunRestartRecovery `
  -RunLongSoak `
  -LongSoakCount 100 `
  -LongSoakDelayMs 1000
```

- [ ] **Step 3: Add Provider Gate Matrix to release readiness**

In `docs/runtime/release-readiness.md`, add a table:

```markdown
## Provider Gate Matrix

| Provider | Required before release claim | Long stress required | Notes |
|---|---:|---:|---|
| OpenAI-compatible official API | Yes when claiming official API readiness | Yes when quota allows | Includes relay/gateway-compatible surface. |
| OpenAI-compatible relay/gateway | Yes when claiming relay/gateway readiness | Yes when quota allows | Record gateway model inventory or `-SkipModelInventory` reason. |
| Anthropic | Yes when claiming Anthropic readiness | Optional unless target release advertises Anthropic as verified | Native Messages API path. |
| Ollama | Yes when claiming local-model readiness | Optional but recommended | Requires local Ollama server and pulled model. |
```

- [ ] **Step 4: Run doc hygiene tests**

Run:

```powershell
cargo test --test code_hygiene
```

Expected: PASS.

## Task 11: Real Provider Verification Pass

**Files:**
- No source changes unless failures reveal bugs.

- [ ] **Step 1: Run deterministic checks**

Run:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
cd web-ui
pnpm test
pnpm typecheck
pnpm build
cd ..
```

Expected: all exit code 0.

- [ ] **Step 2: Run OpenAI-compatible full gate**

Use the current configured provider or gateway:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai-compatible `
  -ApiBase "https://<provider-or-gateway>/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "<model-id>"
```

Expected: `evidence-summary.json` records:

```json
{
  "gates": {
    "model_inventory": "pass",
    "provider_smoke": "pass",
    "provider_full_api": "pass",
    "web_provider": "pass",
    "stress": "skipped",
    "external_mcp": "skipped"
  }
}
```

- [ ] **Step 3: Run OpenAI-compatible stress with restart**

Run:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai-compatible `
  -ApiBase "https://<provider-or-gateway>/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "<model-id>" `
  -RunStress `
  -RunRestartRecovery `
  -StressSequentialCount 20 `
  -StressConcurrentCount 5
```

Expected:

- `stress-summary.json` exists;
- `restart_recovery` is `pass`;
- no job has classification other than `pass`;
- `/runs` after restart contains every stress `run_id`.

- [ ] **Step 4: Run Anthropic gate when credentials are present**

Run:

```powershell
if ($env:ANTHROPIC_API_KEY) {
  powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
    -Provider anthropic `
    -ApiBase "https://api.anthropic.com" `
    -ApiKeyEnv ANTHROPIC_API_KEY `
    -Model "claude-3-5-haiku-latest"
}
```

Expected: pass or classified external failure. A 401/403 is configuration, 429 is quota, unsupported tool use is model capability, not a rove runtime defect unless the trace shows request/stream parsing failure.

- [ ] **Step 5: Run Ollama gate when local server is available**

Run:

```powershell
ollama list
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider ollama `
  -ApiBase "http://localhost:11434" `
  -Model "llama3.2"
```

Expected: pass or classified local availability/model failure.

- [ ] **Step 6: Run secret and artifact checks**

Run:

```powershell
git status --short
git diff --check
$diff = git diff -- . ':!tests/api.rs' ':!web-ui/lib/rove-client.test.ts'
$hits = $diff | Select-String -Pattern 'sk-[A-Za-z0-9_-]{16,}|Bearer\s+[A-Za-z0-9._-]{20,}|[A-Za-z0-9_-]{32,}\.[A-Za-z0-9_-]{16,}\.[A-Za-z0-9_-]{16,}' -AllMatches
if ($hits) { $hits | ForEach-Object { $_.Line }; exit 1 } else { "NO_SECRET_PATTERN_HITS_IN_TRACKED_DIFF" }
```

Expected:

- only intentional source/doc changes appear;
- no generated artifacts are tracked;
- secret scan prints `NO_SECRET_PATTERN_HITS_IN_TRACKED_DIFF`.

## Task 12: Final Commit

**Files:**
- All modified implementation, test, and doc files.

- [ ] **Step 1: Review final diff**

Run:

```powershell
git diff --stat
git diff -- scripts/provider-integration.ps1
git diff -- docs/runtime/provider-smoke.md docs/runtime/release-readiness.md
```

Expected: diff only covers provider gate expansion, stress/restart/soak behavior, tests, and docs.

- [ ] **Step 2: Commit**

Run:

```powershell
git add scripts/provider-integration.ps1 .env.integration.example tests/code_hygiene.rs docs/runtime/provider-smoke.md docs/runtime/integration-testing.md docs/runtime/full-integration-runbook.md docs/runtime/release-readiness.md
git commit -m "Expand provider gates and stress coverage"
```

Expected: commit succeeds.

- [ ] **Step 3: Push**

Run:

```powershell
git push origin main
```

Expected: push succeeds and GitHub CI starts.

## Operational Run Profiles

Use these profiles after implementation.

### Fast Provider Gate

Purpose: prove one provider works end-to-end without stress.

```powershell
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai-compatible `
  -ApiBase "https://<provider-or-gateway>/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "<model-id>"
```

Expected duration: 2-5 minutes depending on provider latency.

### Release Provider Gate

Purpose: prove full provider + API/Web + short stress + restart.

```powershell
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai-compatible `
  -ApiBase "https://<provider-or-gateway>/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "<model-id>" `
  -RunStress `
  -RunRestartRecovery `
  -RunExternalMcp `
  -StressSequentialCount 20 `
  -StressConcurrentCount 5
```

Expected duration: 10-30 minutes depending on provider latency and quota.

### Long Soak

Purpose: catch provider/runtime state leaks, queueing issues, and state persistence problems over a longer period.

```powershell
powershell -ExecutionPolicy Bypass -File scripts/provider-integration.ps1 `
  -Provider openai-compatible `
  -ApiBase "https://<provider-or-gateway>/v1" `
  -ApiKeyEnv OPENAI_API_KEY `
  -Model "<model-id>" `
  -RunStress `
  -RunRestartRecovery `
  -RunLongSoak `
  -LongSoakCount 100 `
  -LongSoakDelayMs 1000 `
  -StressSequentialCount 20 `
  -StressConcurrentCount 5
```

Expected duration: 45-120 minutes depending on provider latency and rate limits.

## Failure Classification Rules

Use this table in final reports:

| Signal | Classification | Release Meaning |
|---|---|---|
| HTTP 401/403, invalid key, unauthorized | `key/configuration` | Not a rove runtime blocker unless docs/env handling is wrong. |
| HTTP 429, quota, rate limit | `quota/rate limit` | Not a rove runtime blocker; rerun with lower counts or later. |
| DNS/connect timeout | `network/connectivity` | External environment issue unless rove fails to classify it. |
| Model answers text but refuses/misses tool use | `model tool-use/follow-up behavior` | Try another model before calling it a runtime blocker. |
| API process panic, lost run ids, SQLite corruption, missing reports | `rove runtime defect` | Release blocker. |
| Web cannot submit provider profile or report mismatches API | `rove Web/API defect` | Release blocker for Web workbench readiness. |
| Secret appears in artifact or tracked diff | `security defect` | Release blocker. Rotate leaked key if real. |

## Final Acceptance Checklist

Before calling this work done:

- [ ] OpenAI-compatible full gate passes after the changes.
- [ ] Anthropic full gate passes when credentials are available, or has a classified external failure artifact.
- [ ] Ollama full gate passes when local server/model are available, or has a classified local availability artifact.
- [ ] Stress with restart recovery passes for at least one real OpenAI-compatible provider.
- [ ] Long soak has been run once before release readiness is claimed, or explicitly deferred with reason.
- [ ] `evidence-summary.json` includes every requested gate and no raw secret.
- [ ] `stress-summary.json` includes sequential, concurrent, restart, and long-soak fields.
- [ ] Docs show exact commands for fast, release, and long-soak profiles.
- [ ] CI passes after push.
