# MVP Readiness Follow-Through Implementation Plan

> **For implementers:** Execute this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make rove easier to verify and start as a real local MVP by fixing integration port handling, adding a one-command dev launcher, documenting readiness/security boundaries, and running deterministic plus provider smoke gates.

**Architecture:** Keep runtime code unchanged. Improve the developer-facing shell and test harness around the existing CLI/API/Web architecture, then document the verification and release posture. Scripts own process lifecycle; Playwright derives its URL from environment; docs remain under existing runtime documentation.

**Tech Stack:** PowerShell, Rust/Cargo, Next.js/Playwright/pnpm, Markdown docs.

---

## File Map

- Modify `web-ui/playwright.config.ts`: derive base URL and web server port from env.
- Modify `scripts/integration-smoke.ps1`: export Playwright URL/port to match runner-selected Web port.
- Create `scripts/dev.ps1`: one-command local API + Web launcher.
- Modify `README.md`: add one-command quick start and fake/provider notes.
- Modify `web-ui/README.md`: document configurable port and proxy base.
- Modify `docs/runtime/integration-testing.md`: document custom-port local-full usage.
- Modify `docs/runtime/full-integration-runbook.md`: mention the launcher and provider artifact expectations.
- Create `docs/runtime/release-readiness.md`: release/security checklist.

## Task 1: Port-configurable Playwright and Integration Runner

**Files:**
- Modify: `web-ui/playwright.config.ts`
- Modify: `scripts/integration-smoke.ps1`

- [ ] **Step 1: Reproduce the current custom-port failure**

Run:

```powershell
$root = Join-Path $env:TEMP ("rove-integration-red-" + [guid]::NewGuid().ToString("N"))
powershell -ExecutionPolicy Bypass -File scripts\integration-smoke.ps1 -ApiAddr "127.0.0.1:18788" -WebPort "13000" -IntegrationRoot $root
```

Expected before the fix: FAIL because Playwright/Next still uses a fixed `localhost:3000` or conflicts with the runner-started server.

- [ ] **Step 2: Update Playwright config**

Replace `web-ui/playwright.config.ts` with logic equivalent to:

```typescript
import { defineConfig, devices } from "@playwright/test";

const webPort = process.env.ROVE_WEB_PORT || "3000";
const baseURL = process.env.PLAYWRIGHT_BASE_URL || `http://localhost:${webPort}`;
const webServerCommand = `pnpm exec next dev --port ${webPort}`;

export default defineConfig({
  testDir: "./tests/e2e",
  timeout: 30_000,
  expect: {
    timeout: 10_000,
  },
  use: {
    baseURL,
    trace: "retain-on-failure",
  },
  webServer: {
    command: webServerCommand,
    url: baseURL,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
});
```

- [ ] **Step 3: Update integration runner environment**

In `scripts/integration-smoke.ps1`, before invoking Playwright, set:

```powershell
$env:ROVE_WEB_PORT = $WebPort
$env:PLAYWRIGHT_BASE_URL = $WebBase
```

Keep the existing runner-owned Web process startup. With `reuseExistingServer: !process.env.CI`, Playwright should reuse that server when `PLAYWRIGHT_BASE_URL` points to the same URL.

- [ ] **Step 4: Verify custom-port local-full now passes**

Run:

```powershell
$root = Join-Path $env:TEMP ("rove-integration-custom-" + [guid]::NewGuid().ToString("N"))
powershell -ExecutionPolicy Bypass -File scripts\integration-smoke.ps1 -ApiAddr "127.0.0.1:18788" -WebPort "13000" -IntegrationRoot $root
```

Expected: PASS, including `3 passed` from `real-api.spec.ts` and `local-full integration smoke completed`.

## Task 2: One-command Development Launcher

**Files:**
- Create: `scripts/dev.ps1`

- [ ] **Step 1: Create launcher script**

Create `scripts/dev.ps1` with these responsibilities:

```powershell
param(
    [string]$ApiAddr = $(if ($env:ROVE_API_BIND_ADDR) { $env:ROVE_API_BIND_ADDR } else { "127.0.0.1:8787" }),
    [string]$WebPort = $(if ($env:ROVE_WEB_PORT) { $env:ROVE_WEB_PORT } else { "3000" }),
    [string]$Workspace = $(if ($env:ROVE_DEV_WORKSPACE) { $env:ROVE_DEV_WORKSPACE } else { (Split-Path -Parent $PSScriptRoot) }),
    [switch]$Provider,
    [switch]$InstallWebDeps
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Import-DotEnv([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return }
    Get-Content -LiteralPath $Path | ForEach-Object {
        $line = $_.Trim()
        if (-not $line -or $line.StartsWith("#")) { return }
        $parts = $line.Split("=", 2)
        if ($parts.Count -ne 2) { return }
        $name = $parts[0].Trim()
        $value = $parts[1].Trim()
        if ($value.Length -ge 2 -and (($value.StartsWith('"') -and $value.EndsWith('"')) -or ($value.StartsWith("'") -and $value.EndsWith("'")))) {
            $value = $value.Substring(1, $value.Length - 2)
        }
        if (-not [Environment]::GetEnvironmentVariable($name, "Process")) {
            [Environment]::SetEnvironmentVariable($name, $value, "Process")
        }
    }
}

function Test-CommandAvailable([string]$Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required command '$Name' was not found on PATH."
    }
}

function Test-PortFree([string]$Address, [string]$Name) {
    $portText = $Address.Split(":")[-1]
    $port = [int]$portText
    $existing = Get-NetTCPConnection -LocalPort $port -ErrorAction SilentlyContinue | Where-Object { $_.State -eq "Listen" }
    if ($existing) {
        throw "$Name port $port is already in use. Stop the existing process or pass a different port."
    }
}

function Start-BackgroundCommand(
    [string]$Command,
    [string[]]$Arguments,
    [string]$WorkingDirectory
) {
    $resolved = Get-Command $Command -ErrorAction Stop
    if ($resolved.CommandType -eq "ExternalScript") {
        $argumentList = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $resolved.Path) + $Arguments
        return Start-Process -FilePath "powershell" -ArgumentList $argumentList -WorkingDirectory $WorkingDirectory -PassThru -WindowStyle Hidden
    }
    return Start-Process -FilePath $resolved.Path -ArgumentList $Arguments -WorkingDirectory $WorkingDirectory -PassThru -WindowStyle Hidden
}

function Stop-ProcessTree([System.Diagnostics.Process]$Process) {
    if ($null -eq $Process -or $Process.HasExited) { return }
    $children = Get-CimInstance Win32_Process -Filter "ParentProcessId = $($Process.Id)" -ErrorAction SilentlyContinue
    foreach ($child in $children) {
        $childProcess = Get-Process -Id $child.ProcessId -ErrorAction SilentlyContinue
        if ($childProcess) { Stop-ProcessTree $childProcess }
    }
    Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue
}

function Wait-HttpOk([string]$Uri, [int]$TimeoutSeconds, [string]$Name) {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        try {
            $response = Invoke-WebRequest -UseBasicParsing -Uri $Uri -TimeoutSec 3
            if ($response.StatusCode -ge 200 -and $response.StatusCode -lt 500) { return }
        } catch {
            Start-Sleep -Milliseconds 500
        }
    }
    throw "Timed out waiting for $Name at $Uri."
}

$RepoRoot = Split-Path -Parent $PSScriptRoot
Import-DotEnv (Join-Path $RepoRoot ".env.integration")

Test-CommandAvailable "cargo"
Test-CommandAvailable "pnpm"
Test-PortFree $ApiAddr "API"
Test-PortFree "127.0.0.1:$WebPort" "Web"

$Workspace = [System.IO.Path]::GetFullPath($Workspace)
New-Item -ItemType Directory -Force -Path $Workspace | Out-Null

if (-not $Provider) {
    $env:ROVE_PROVIDER = "fake"
    $env:ROVE_MODEL = "fake"
}

$env:ROVE_API_BIND_ADDR = $ApiAddr
$env:ROVE_API_BASE = "http://$ApiAddr"
$env:ROVE_WEB_PORT = $WebPort
$env:PLAYWRIGHT_BASE_URL = "http://localhost:$WebPort"

if ($InstallWebDeps) {
    Push-Location (Join-Path $RepoRoot "web-ui")
    try { pnpm install --frozen-lockfile } finally { Pop-Location }
}

$apiProcess = $null
$webProcess = $null

try {
    $apiProcess = Start-BackgroundCommand -Command "cargo" -Arguments @("run", "--bin", "rove-api", "--", "--addr", $ApiAddr, "-C", $Workspace) -WorkingDirectory $RepoRoot
    Wait-HttpOk -Uri "http://$ApiAddr/runs?limit=1" -TimeoutSeconds 60 -Name "rove-api"

    $webProcess = Start-BackgroundCommand -Command "pnpm" -Arguments @("exec", "next", "dev", "--port", $WebPort) -WorkingDirectory (Join-Path $RepoRoot "web-ui")
    Wait-HttpOk -Uri "http://localhost:$WebPort" -TimeoutSeconds 120 -Name "web-ui"

    Write-Host "rove dev environment is running"
    Write-Host "Web:       http://localhost:$WebPort"
    Write-Host "API:       http://$ApiAddr"
    Write-Host "Workspace: $Workspace"
    Write-Host "Provider:  $env:ROVE_PROVIDER"
    Write-Host "Model:     $env:ROVE_MODEL"
    Write-Host "Press Ctrl+C to stop API and Web."

    while ($true) { Start-Sleep -Seconds 1 }
} finally {
    if ($webProcess) { Stop-ProcessTree $webProcess }
    if ($apiProcess) { Stop-ProcessTree $apiProcess }
}
```

- [ ] **Step 2: Smoke-test launcher on alternate ports**

Run it briefly:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\dev.ps1 -ApiAddr "127.0.0.1:18789" -WebPort "13001"
```

Expected: script prints Web/API URLs. Stop with Ctrl+C and confirm both ports are released:

```powershell
Get-NetTCPConnection -LocalPort 18789 -ErrorAction SilentlyContinue
Get-NetTCPConnection -LocalPort 13001 -ErrorAction SilentlyContinue
```

Expected: no listening connections.

## Task 3: Documentation Updates

**Files:**
- Modify: `README.md`
- Modify: `web-ui/README.md`
- Modify: `docs/runtime/integration-testing.md`
- Modify: `docs/runtime/full-integration-runbook.md`
- Create: `docs/runtime/release-readiness.md`

- [ ] **Step 1: Update root README Quick Start**

Add `scripts/dev.ps1` as the first Web/API path and keep manual commands as explicit alternatives. Include fake and provider examples:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\dev.ps1

$env:ROVE_PROVIDER = "openai-compatible"
$env:ROVE_MODEL = "Qwen/Qwen3-Coder-30B-A3B-Instruct"
$env:OPENAI_API_BASE = "https://api.siliconflow.cn/v1"
$env:OPENAI_API_KEY = "<secret>"
powershell -ExecutionPolicy Bypass -File scripts\dev.ps1 -Provider
```

- [ ] **Step 2: Update Web README**

Document:

```powershell
$env:ROVE_WEB_PORT = "3001"
$env:ROVE_API_BASE = "http://127.0.0.1:8787"
pnpm exec next dev --port $env:ROVE_WEB_PORT
```

Also note that Playwright uses `PLAYWRIGHT_BASE_URL` or `ROVE_WEB_PORT`.

- [ ] **Step 3: Update integration testing docs**

Add custom-port examples:

```powershell
$root = Join-Path $env:TEMP "rove-integration-custom"
powershell -ExecutionPolicy Bypass -File scripts\integration-smoke.ps1 -ApiAddr "127.0.0.1:18788" -WebPort "13000" -IntegrationRoot $root
```

- [ ] **Step 4: Update full integration runbook**

Add a short "Developer launch" section pointing to `scripts/dev.ps1`, and clarify that full provider/stress evidence remains separate from the local dev launcher.

- [ ] **Step 5: Add release readiness checklist**

Create `docs/runtime/release-readiness.md` with sections:

- Deterministic gates
- Local-full integration
- Provider smoke
- External tools/RAG
- Stress and restart recovery
- Packaging notes
- Security posture
- Out-of-scope reminders
- Final evidence package

## Task 4: Provider Inventory and Smoke

**Files:**
- No tracked file changes required.

- [ ] **Step 1: Query SiliconFlow model inventory when key exists**

Run:

```powershell
if (-not $env:SILICONFLOW_API_KEY) {
  . .\.env.integration
}
$headers = @{ Authorization = "Bearer $env:SILICONFLOW_API_KEY" }
$models = Invoke-RestMethod -Uri "https://api.siliconflow.cn/v1/models" -Headers $headers
$nonPro = $models.data | Where-Object { $_.id -and -not $_.id.StartsWith("Pro/") } | Sort-Object id
$artifactRoot = Join-Path $env:TEMP "rove-provider-smoke"
New-Item -ItemType Directory -Force -Path $artifactRoot | Out-Null
$nonPro | Select-Object id, owned_by | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $artifactRoot "siliconflow-non-pro-models.json")
```

Expected: command returns HTTP 200 and saves non-secret model inventory.

- [ ] **Step 2: Select smoke model**

Prefer `$env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL` if it exists in `$nonPro.id`; otherwise select the first available preferred candidate:

```powershell
$preferred = @(
  $env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL,
  "Qwen/Qwen3-Coder-30B-A3B-Instruct",
  "Qwen/Qwen3-32B",
  "deepseek-ai/DeepSeek-V3.2",
  "deepseek-ai/DeepSeek-V3"
) | Where-Object { $_ }
$selected = $preferred | Where-Object { $nonPro.id -contains $_ } | Select-Object -First 1
if (-not $selected) { $selected = ($nonPro | Select-Object -First 1).id }
$selected | Set-Content -LiteralPath (Join-Path $artifactRoot "selected-model.txt")
```

- [ ] **Step 3: Run provider smoke**

Run:

```powershell
$env:ROVE_PROVIDER_SMOKE_OPENAI = "1"
$env:OPENAI_API_KEY = $env:SILICONFLOW_API_KEY
$env:OPENAI_API_BASE = "https://api.siliconflow.cn/v1"
$env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL = $selected
cargo test --test provider_smoke openai_compatible_real_provider_smoke_when_enabled -- --exact --nocapture
```

Expected: PASS. If it fails, save the terminal output classification in the final report without exposing the key.

## Task 5: Full Verification

**Files:**
- No new source files beyond previous tasks.

- [ ] **Step 1: Run deterministic Rust checks**

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Expected: all exit code 0.

- [ ] **Step 2: Run deterministic Web checks**

```powershell
cd web-ui
pnpm test
pnpm typecheck
pnpm build
cd ..
```

Expected: all exit code 0. If `web-ui/next-env.d.ts` is rewritten by Next, inspect and restore only generated-reference churn if appropriate.

- [ ] **Step 3: Run local-full integration on default and custom ports**

```powershell
$rootDefault = Join-Path $env:TEMP ("rove-integration-default-" + [guid]::NewGuid().ToString("N"))
powershell -ExecutionPolicy Bypass -File scripts\integration-smoke.ps1 -IntegrationRoot $rootDefault

$rootCustom = Join-Path $env:TEMP ("rove-integration-custom-" + [guid]::NewGuid().ToString("N"))
powershell -ExecutionPolicy Bypass -File scripts\integration-smoke.ps1 -ApiAddr "127.0.0.1:18788" -WebPort "13000" -IntegrationRoot $rootCustom
```

Expected: both pass and print artifact directories.

- [ ] **Step 4: Check worktree**

```powershell
git status --short
```

Expected: only intentional source/doc changes are present; no secrets, logs, state, screenshots, SQLite files, or generated runtime artifacts are tracked.
