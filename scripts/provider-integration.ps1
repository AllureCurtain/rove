param(
    [string]$Provider = $(if ($env:ROVE_PROVIDER_INTEGRATION_PROVIDER) { $env:ROVE_PROVIDER_INTEGRATION_PROVIDER } else { "openai-compatible" }),
    [string]$Model = $(if ($env:ROVE_PROVIDER_INTEGRATION_MODEL) { $env:ROVE_PROVIDER_INTEGRATION_MODEL } elseif ($env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL) { $env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL } else { "" }),
    [string]$ApiBase = $(if ($env:ROVE_PROVIDER_INTEGRATION_API_BASE) { $env:ROVE_PROVIDER_INTEGRATION_API_BASE } elseif ($env:OPENAI_API_BASE) { $env:OPENAI_API_BASE } else { "https://api.openai.com/v1" }),
    [string]$ApiKeyEnv = $(if ($env:ROVE_PROVIDER_INTEGRATION_API_KEY_ENV) { $env:ROVE_PROVIDER_INTEGRATION_API_KEY_ENV } else { "OPENAI_API_KEY" }),
    [string]$ModelsEndpoint = $(if ($env:ROVE_PROVIDER_INTEGRATION_MODELS_ENDPOINT) { $env:ROVE_PROVIDER_INTEGRATION_MODELS_ENDPOINT } else { "" }),
    [string]$IntegrationRoot = $(if ($env:ROVE_PROVIDER_INTEGRATION_ROOT) { $env:ROVE_PROVIDER_INTEGRATION_ROOT } else { (Join-Path $env:TEMP ("rove-provider-integration-" + [guid]::NewGuid().ToString("N"))) }),
    [string]$ApiAddr = $(if ($env:ROVE_PROVIDER_INTEGRATION_API_ADDR) { $env:ROVE_PROVIDER_INTEGRATION_API_ADDR } else { "127.0.0.1:18791" }),
    [string]$WebPort = $(if ($env:ROVE_PROVIDER_INTEGRATION_WEB_PORT) { $env:ROVE_PROVIDER_INTEGRATION_WEB_PORT } else { "13021" }),
    [switch]$SkipModelInventory,
    [switch]$SkipProviderSmoke,
    [switch]$SkipApiSmoke,
    [switch]$SkipWebSmoke,
    [switch]$RunStress,
    [switch]$RunExternalMcp,
    [int]$StressSequentialCount = $(if ($env:ROVE_PROVIDER_INTEGRATION_STRESS_SEQUENTIAL_COUNT) { [int]$env:ROVE_PROVIDER_INTEGRATION_STRESS_SEQUENTIAL_COUNT } else { 5 }),
    [int]$StressConcurrentCount = $(if ($env:ROVE_PROVIDER_INTEGRATION_STRESS_CONCURRENT_COUNT) { [int]$env:ROVE_PROVIDER_INTEGRATION_STRESS_CONCURRENT_COUNT } else { 3 }),
    [string]$ExternalMcpToolName = $(if ($env:ROVE_PROVIDER_INTEGRATION_EXTERNAL_MCP_TOOL) { $env:ROVE_PROVIDER_INTEGRATION_EXTERNAL_MCP_TOOL } else { "mcp__mock_server__echo_remote" }),
    [switch]$KeepState
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Import-DotEnvNoOverride([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }
    Get-Content -LiteralPath $Path | ForEach-Object {
        $line = $_.Trim()
        if (-not $line -or $line.StartsWith("#")) {
            return
        }
        $parts = $line.Split("=", 2)
        if ($parts.Count -ne 2) {
            return
        }
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

function Resolve-RepoPath([string]$Path) {
    if ([System.IO.Path]::IsPathRooted($Path)) {
        return $Path
    }
    return (Join-Path $RepoRoot $Path)
}

function Test-CommandAvailable([string]$Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required command '$Name' was not found on PATH."
    }
}

function Start-BackgroundCommand(
    [string]$Command,
    [string[]]$Arguments,
    [string]$WorkingDirectory,
    [string]$StdoutLog,
    [string]$StderrLog
) {
    $resolved = Get-Command $Command -ErrorAction Stop
    if ($resolved.CommandType -eq "ExternalScript") {
        $argumentList = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $resolved.Path) + $Arguments
        return Start-Process -FilePath "powershell" -ArgumentList $argumentList -WorkingDirectory $WorkingDirectory -RedirectStandardOutput $StdoutLog -RedirectStandardError $StderrLog -PassThru -WindowStyle Hidden
    }
    return Start-Process -FilePath $resolved.Path -ArgumentList $Arguments -WorkingDirectory $WorkingDirectory -RedirectStandardOutput $StdoutLog -RedirectStandardError $StderrLog -PassThru -WindowStyle Hidden
}

function Stop-ProcessTree([System.Diagnostics.Process]$Process) {
    if ($null -eq $Process -or $Process.HasExited) {
        return
    }
    try {
        $children = Get-CimInstance Win32_Process -Filter "ParentProcessId = $($Process.Id)" -ErrorAction SilentlyContinue
        foreach ($child in $children) {
            $childProcess = Get-Process -Id $child.ProcessId -ErrorAction SilentlyContinue
            if ($childProcess) {
                Stop-ProcessTree $childProcess
            }
        }
        Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue
    } catch {
        Write-Warning "Failed to stop process $($Process.Id): $($_.Exception.Message)"
    }
}

function Wait-HttpOk([string]$Uri, [int]$TimeoutSeconds, [string]$Name) {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $lastError = $null
    while ((Get-Date) -lt $deadline) {
        try {
            $response = Invoke-WebRequest -UseBasicParsing -Uri $Uri -TimeoutSec 3
            if ($response.StatusCode -ge 200 -and $response.StatusCode -lt 500) {
                return
            }
        } catch {
            $lastError = $_.Exception.Message
        }
        Start-Sleep -Milliseconds 500
    }
    throw "Timed out waiting for $Name at $Uri. Last error: $lastError"
}

function Invoke-Json([string]$Method, [string]$Uri, [object]$Body = $null) {
    if ($null -eq $Body) {
        return Invoke-RestMethod -Method $Method -Uri $Uri -TimeoutSec 30
    }
    $json = $Body | ConvertTo-Json -Depth 20 -Compress
    return Invoke-RestMethod -Method $Method -Uri $Uri -ContentType "application/json" -Body $json -TimeoutSec 30
}

function Wait-JobTerminal([string]$JobId, [string]$Name, [int]$TimeoutSeconds = 180) {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $lastState = $null
    while ((Get-Date) -lt $deadline) {
        $lastState = Invoke-Json -Method Get -Uri "$ApiBaseLocal/jobs/$JobId/state"
        if ($lastState.status -in @("done", "error", "cancelled", "interrupted")) {
            $lastState | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "provider-full-$Name.state.json")
            return $lastState
        }
        Start-Sleep -Milliseconds 750
    }
    $snapshot = if ($null -ne $lastState) { $lastState | ConvertTo-Json -Depth 20 -Compress } else { "<no state>" }
    throw "Timed out waiting for $Name job $JobId. Last state: $snapshot"
}

function Get-ApiKeyValue {
    $value = [Environment]::GetEnvironmentVariable($ApiKeyEnv, "Process")
    if (-not $value -and $ApiKeyEnv -ne "OPENAI_API_KEY") {
        $value = [Environment]::GetEnvironmentVariable("OPENAI_API_KEY", "Process")
    }
    if (-not $value) {
        throw "Provider API key is not set. Expected environment variable '$ApiKeyEnv'."
    }
    return $value
}

function Set-ProviderEnvironment {
    $key = Get-ApiKeyValue
    if ($Provider -ne "openai-compatible") {
        throw "provider-integration.ps1 currently automates API/Web gates for openai-compatible providers. Use provider_smoke tests for '$Provider'."
    }
    $env:ROVE_PROVIDER = $Provider
    $env:ROVE_MODEL = $Model
    $env:OPENAI_API_KEY = $key
    $env:OPENAI_API_BASE = $ApiBase
}

function Get-DefaultModelsEndpoint {
    if ($ModelsEndpoint) {
        return $ModelsEndpoint
    }
    return ($ApiBase.TrimEnd("/") + "/models")
}

function Invoke-ModelInventory {
    if ($SkipModelInventory) {
        return "skipped"
    }
    $endpoint = Get-DefaultModelsEndpoint
    $headers = @{ Authorization = "Bearer $(Get-ApiKeyValue)" }
    $models = Invoke-RestMethod -Uri $endpoint -Headers $headers -TimeoutSec 30
    $items = @($models.data)
    $visible = $items | Where-Object { $_.id } | Sort-Object id
    $visible | Select-Object id, owned_by | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "provider-models.json")
    $Model | Set-Content -LiteralPath (Join-Path $ArtifactsDir "selected-provider-model.txt")

    if (-not ($visible | Where-Object { $_.id -eq $Model } | Select-Object -First 1)) {
        throw "Model '$Model' was not present in provider model inventory from $endpoint."
    }
    return "pass"
}

function Invoke-ProviderSmoke {
    if ($SkipProviderSmoke) {
        return "skipped"
    }
    $env:ROVE_PROVIDER_SMOKE_OPENAI = "1"
    $env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL = $Model
    $log = Join-Path $ArtifactsDir "provider-smoke.log"
    $previousErrorActionPreference = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & cargo test --test provider_smoke openai_compatible_real_provider_smoke_when_enabled -- --exact --nocapture 2>&1
        $exit = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    $output | Set-Content -LiteralPath $log
    $classification = Classify-ProviderOutput -ExitCode $exit -Text ($output -join "`n")
    @{
        model = $Model
        exit_code = $exit
        classification = $classification
        log = $log
    } | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "provider-smoke-result.json")
    if ($exit -ne 0) {
        throw "provider smoke failed with classification '$classification'. See $log"
    }
    return $classification
}

function Classify-ProviderOutput([int]$ExitCode, [string]$Text) {
    if ($ExitCode -eq 0) {
        return "pass"
    }
    if ($Text -match "401|403|Unauthorized|Invalid token|invalid api key") {
        return "key/configuration"
    }
    if ($Text -match "429|rate limit|quota") {
        return "quota/rate limit"
    }
    if ($Text -match "did not emit an echo tool call|did not complete echo tool output|unexpected provider smoke output|tool-use|tool use") {
        return "model tool-use/follow-up behavior"
    }
    if ($Text -match "Connect|timed out|timeout|connection|dns") {
        return "network/connectivity"
    }
    return "rove runtime or assertion failure"
}

function Invoke-ApiSmoke {
    if ($SkipApiSmoke) {
        return "skipped"
    }
    $apiProcess = $null
    try {
        $apiProcess = Start-BackgroundCommand -Command "cargo" -Arguments @("run", "--bin", "rove-api", "--", "--addr", $ApiAddr, "-C", $WorkspaceDir) -WorkingDirectory $RepoRoot -StdoutLog (Join-Path $ArtifactsDir "provider-full-api.out.log") -StderrLog (Join-Path $ArtifactsDir "provider-full-api.err.log")
        Wait-HttpOk -Uri "$ApiBaseLocal/runs?limit=1" -TimeoutSeconds 90 -Name "rove-api"

        $plain = Invoke-Json -Method Post -Uri "$ApiBaseLocal/jobs" -Body @{
            message = "Reply with exactly: rove provider api plain ok"
            model = $Model
            approval = "auto"
            max_steps = 4
        }
        $plain | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "provider-full-plain.created.json")
        $plainState = Wait-JobTerminal -JobId $plain.job_id -Name "plain"
        $plainReport = Invoke-Json -Method Get -Uri "$ApiBaseLocal/runs/$($plain.run_id)/report"
        $plainReport | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "provider-full-plain.report.json")

        $tool = Invoke-Json -Method Post -Uri "$ApiBaseLocal/jobs" -Body @{
            message = "Use the echo tool exactly once with message `"rove provider api tool ok`", then reply with exactly: rove provider api done"
            model = $Model
            approval = "auto"
            max_steps = 4
        }
        $tool | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "provider-full-tool.created.json")
        $toolState = Wait-JobTerminal -JobId $tool.job_id -Name "tool"
        $toolReport = Invoke-Json -Method Get -Uri "$ApiBaseLocal/runs/$($tool.run_id)/report"
        $toolReport | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "provider-full-tool.report.json")

        $runs = Invoke-Json -Method Get -Uri "$ApiBaseLocal/runs?limit=25"
        $runs | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "provider-full-runs.json")

        if ($plainState.status -ne "done" -or $plainReport.status -ne "success") {
            throw "plain API provider run did not complete successfully"
        }
        if ($toolState.status -ne "done" -or $toolReport.status -ne "success" -or [int]$toolReport.tool_calls -lt 1 -or [int]$toolReport.tool_failures -ne 0) {
            throw "tool API provider run did not complete successful echo tool use"
        }

        @{
            plain = @{
                run_id = [string]$plain.run_id
                status = [string]$plainState.status
                report_status = [string]$plainReport.status
                reason = [string]$plainReport.termination_reason
                output = [string]$plainReport.output
                tool_calls = [int]$plainReport.tool_calls
            }
            tool = @{
                run_id = [string]$tool.run_id
                status = [string]$toolState.status
                report_status = [string]$toolReport.status
                reason = [string]$toolReport.termination_reason
                output = [string]$toolReport.output
                tool_calls = [int]$toolReport.tool_calls
                tool_failures = [int]$toolReport.tool_failures
            }
        } | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "provider-full-api-summary.json")

        return "pass"
    } finally {
        if ($apiProcess) {
            Stop-ProcessTree $apiProcess
        }
    }
}

function Invoke-WebSmoke {
    if ($SkipWebSmoke) {
        return "skipped"
    }
    $apiProcess = $null
    $webProcess = $null
    try {
        $apiProcess = Start-BackgroundCommand -Command "cargo" -Arguments @("run", "--bin", "rove-api", "--", "--addr", $ApiAddr, "-C", $WebWorkspaceDir) -WorkingDirectory $RepoRoot -StdoutLog (Join-Path $ArtifactsDir "web-provider-api.out.log") -StderrLog (Join-Path $ArtifactsDir "web-provider-api.err.log")
        Wait-HttpOk -Uri "$ApiBaseLocal/runs?limit=1" -TimeoutSeconds 90 -Name "rove-api"

        $env:ROVE_API_BASE = $ApiBaseLocal
        $env:ROVE_WEB_PORT = $WebPort
        $env:PLAYWRIGHT_BASE_URL = $WebBase
        $webProcess = Start-BackgroundCommand -Command "pnpm" -Arguments @("exec", "next", "dev", "--port", $WebPort) -WorkingDirectory (Join-Path $RepoRoot "web-ui") -StdoutLog (Join-Path $ArtifactsDir "web-provider-web.out.log") -StderrLog (Join-Path $ArtifactsDir "web-provider-web.err.log")
        Wait-HttpOk -Uri $WebBase -TimeoutSeconds 120 -Name "web-ui"

        $resultPath = Join-Path $ArtifactsDir "web-provider-result.json"
        $screenshotPath = Join-Path $ArtifactsDir "web-provider.png"
        $env:ROVE_WEB_PROVIDER_RESULT = $resultPath
        $env:ROVE_WEB_PROVIDER_SCREENSHOT = $screenshotPath
        $nodeScript = @'
const { chromium } = require('@playwright/test');
const fs = require('fs');
const baseURL = process.env.PLAYWRIGHT_BASE_URL;
const model = process.env.ROVE_MODEL;
const resultPath = process.env.ROVE_WEB_PROVIDER_RESULT;
const screenshotPath = process.env.ROVE_WEB_PROVIDER_SCREENSHOT;
const prompt = 'Use the echo tool exactly once with message "rove provider web ok". After the tool returns, reply only with that exact message.';
(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  await page.goto(baseURL, { waitUntil: 'networkidle' });
  await page.getByLabel('Task').fill(prompt);
  await page.getByLabel('Model').fill(model);
  await page.getByLabel('Steps').fill('4');
  await page.getByRole('button', { name: 'Run' }).click();
  await page.getByLabel('Run summary').getByText('Run completed').first().waitFor({ timeout: 120000 });
  await page.screenshot({ path: screenshotPath, fullPage: true });
  const result = {
    baseURL,
    model,
    prompt,
    title: await page.title(),
    runSummary: await page.getByLabel('Run summary').innerText().catch(() => ''),
    runDetails: await page.getByLabel('Run details').innerText().catch(() => ''),
    messageStream: await page.locator('.message-stream').innerText().catch(() => ''),
  };
  fs.writeFileSync(resultPath, JSON.stringify(result, null, 2));
  await browser.close();
})().catch(async (err) => {
  fs.writeFileSync(resultPath, JSON.stringify({ error: String(err && err.stack || err) }, null, 2));
  process.exit(1);
});
'@
        Push-Location (Join-Path $RepoRoot "web-ui")
        try {
            $nodeScript | pnpm exec node -
            if ($LASTEXITCODE -ne 0) {
                throw "web provider Playwright script failed with exit code $LASTEXITCODE"
            }
        } finally {
            Pop-Location
        }

        $runs = Invoke-Json -Method Get -Uri "$ApiBaseLocal/runs?limit=25"
        $runs | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "web-provider-runs.json")
        $latestRun = $runs.runs | Sort-Object run_id -Descending | Select-Object -First 1
        if (-not $latestRun) {
            throw "no Web provider run found"
        }
        $report = Invoke-Json -Method Get -Uri "$ApiBaseLocal/runs/$($latestRun.run_id)/report"
        $report | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "web-provider-report.json")
        if ($report.status -ne "success" -or [int]$report.tool_calls -lt 1 -or [int]$report.tool_failures -ne 0) {
            throw "Web provider report did not record successful tool use"
        }
        return "pass"
    } finally {
        if ($webProcess) {
            Stop-ProcessTree $webProcess
        }
        if ($apiProcess) {
            Stop-ProcessTree $apiProcess
        }
    }
}

function Invoke-StressGate {
    if (-not $RunStress) {
        return "skipped"
    }
    $apiProcess = $null
    try {
        $stressWorkspace = Join-Path $IntegrationRoot "workspace-stress"
        New-Item -ItemType Directory -Force -Path $stressWorkspace | Out-Null
        $apiProcess = Start-BackgroundCommand -Command "cargo" -Arguments @("run", "--bin", "rove-api", "--", "--addr", $ApiAddr, "-C", $stressWorkspace) -WorkingDirectory $RepoRoot -StdoutLog (Join-Path $ArtifactsDir "stress-api.out.log") -StderrLog (Join-Path $ArtifactsDir "stress-api.err.log")
        Wait-HttpOk -Uri "$ApiBaseLocal/runs?limit=1" -TimeoutSeconds 90 -Name "stress rove-api"

        $created = @()
        for ($i = 1; $i -le $StressSequentialCount; $i++) {
            $job = Invoke-Json -Method Post -Uri "$ApiBaseLocal/jobs" -Body @{
                message = "Reply with exactly: rove provider stress ok $i"
                model = $Model
                approval = "auto"
                max_steps = 4
            }
            $state = Wait-JobTerminal -JobId $job.job_id -Name "stress-sequential-$i" -TimeoutSeconds 180
            if ($state.status -ne "done") {
                throw "sequential stress job $i ended with status $($state.status)"
            }
            $created += @{
                kind = "sequential"
                index = $i
                job_id = [string]$job.job_id
                run_id = [string]$job.run_id
                status = [string]$state.status
            }
        }

        $jobs = @()
        for ($i = 1; $i -le $StressConcurrentCount; $i++) {
            $jobs += Invoke-Json -Method Post -Uri "$ApiBaseLocal/jobs" -Body @{
                message = "Reply with exactly: rove provider concurrent stress ok $i"
                model = $Model
                approval = "auto"
                max_steps = 4
            }
        }
        for ($i = 0; $i -lt $jobs.Count; $i++) {
            $state = Wait-JobTerminal -JobId $jobs[$i].job_id -Name "stress-concurrent-$($i + 1)" -TimeoutSeconds 180
            if ($state.status -ne "done") {
                throw "concurrent stress job $($i + 1) ended with status $($state.status)"
            }
            $created += @{
                kind = "concurrent"
                index = $i + 1
                job_id = [string]$jobs[$i].job_id
                run_id = [string]$jobs[$i].run_id
                status = [string]$state.status
            }
        }

        $runs = Invoke-Json -Method Get -Uri "$ApiBaseLocal/runs?limit=100"
        @{
            sequential_count = $StressSequentialCount
            concurrent_count = $StressConcurrentCount
            jobs = $created
            runs_count = @($runs.runs).Count
        } | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "stress-summary.json")
        $runs | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "stress-runs.json")
        return "pass"
    } finally {
        if ($apiProcess) {
            Stop-ProcessTree $apiProcess
        }
    }
}

function Invoke-ExternalMcpGate {
    if (-not $RunExternalMcp) {
        return "skipped"
    }
    $apiProcess = $null
    $previousMcpConfig = [Environment]::GetEnvironmentVariable("ROVE_MCP_CONFIG", "Process")
    try {
        $mcpWorkspace = Join-Path $IntegrationRoot "workspace-mcp"
        New-Item -ItemType Directory -Force -Path $mcpWorkspace | Out-Null
        $mcpConfigSource = Join-Path $RepoRoot ".rove/mcp_servers.example.json"
        if (-not (Test-Path -LiteralPath $mcpConfigSource)) {
            throw "external MCP gate requires $mcpConfigSource"
        }
        $mcpConfigPath = Join-Path $mcpWorkspace ".rove/mcp_servers.json"
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $mcpConfigPath) | Out-Null
        Copy-Item -LiteralPath $mcpConfigSource -Destination $mcpConfigPath -Force
        Copy-Item -LiteralPath $mcpConfigPath -Destination (Join-Path $ArtifactsDir "external-mcp-config.redacted.json") -Force
        [Environment]::SetEnvironmentVariable("ROVE_MCP_CONFIG", $mcpConfigPath, "Process")
        $apiProcess = Start-BackgroundCommand -Command "cargo" -Arguments @("run", "--bin", "rove-api", "--", "--addr", $ApiAddr, "-C", $mcpWorkspace) -WorkingDirectory $RepoRoot -StdoutLog (Join-Path $ArtifactsDir "external-mcp-api.out.log") -StderrLog (Join-Path $ArtifactsDir "external-mcp-api.err.log")
        Wait-HttpOk -Uri "$ApiBaseLocal/runs?limit=1" -TimeoutSeconds 90 -Name "external MCP rove-api"

        $message = @{
            tool = $ExternalMcpToolName
            args = @{ message = "hello provider mcp" }
        } | ConvertTo-Json -Depth 10 -Compress
        $job = Invoke-Json -Method Post -Uri "$ApiBaseLocal/jobs" -Body @{
            message = $message
            model = "fake-raw"
            approval = "auto"
            max_steps = 1
        }
        $state = Wait-JobTerminal -JobId $job.job_id -Name "external-mcp" -TimeoutSeconds 120
        $report = Invoke-Json -Method Get -Uri "$ApiBaseLocal/runs/$($job.run_id)/report"
        $state | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "external-mcp.state.json")
        $report | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "external-mcp.report.json")
        if ($state.status -ne "done" -or $report.status -ne "success" -or [int]$report.tool_calls -lt 1 -or [int]$report.tool_failures -ne 0) {
            throw "external MCP gate did not record a successful tool call for $ExternalMcpToolName"
        }
        @{
            tool = $ExternalMcpToolName
            job_id = [string]$job.job_id
            run_id = [string]$job.run_id
            status = [string]$state.status
            report_status = [string]$report.status
            output = [string]$report.output
        } | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "external-mcp-summary.json")
        return "pass"
    } finally {
        [Environment]::SetEnvironmentVariable("ROVE_MCP_CONFIG", $previousMcpConfig, "Process")
        if ($apiProcess) {
            Stop-ProcessTree $apiProcess
        }
    }
}

function Write-EvidenceSummary([hashtable]$Gates) {
    @{
        date = (Get-Date).ToString("o")
        git = (& git rev-parse HEAD)
        dirty = (& git status --short)
        artifact_dir = $ArtifactsDir
        provider = $Provider
        api_base = $ApiBase
        model = $Model
        key_env = $ApiKeyEnv
        key_present = [bool](Get-ApiKeyValue)
        gates = $Gates
    } | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "evidence-summary.json")
}

$RepoRoot = Split-Path -Parent $PSScriptRoot
Import-DotEnvNoOverride (Join-Path $RepoRoot ".env.integration")

Test-CommandAvailable "cargo"
Test-CommandAvailable "pnpm"

if (-not $Model) {
    throw "Model is required. Pass -Model or set ROVE_PROVIDER_INTEGRATION_MODEL."
}

$IntegrationRoot = Resolve-RepoPath $IntegrationRoot
$WorkspaceDir = Join-Path $IntegrationRoot "workspace-api"
$WebWorkspaceDir = Join-Path $IntegrationRoot "workspace-web"
$ArtifactsDir = Join-Path $IntegrationRoot "artifacts"
$ApiBaseLocal = "http://$ApiAddr"
$WebBase = "http://127.0.0.1:$WebPort"

if (-not $KeepState -and (Test-Path -LiteralPath $IntegrationRoot)) {
    Remove-Item -LiteralPath $IntegrationRoot -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $WorkspaceDir, $WebWorkspaceDir, $ArtifactsDir | Out-Null

Set-ProviderEnvironment
$env:ROVE_STATE_DIR = ".rove-provider-integration-state"
$env:ROVE_STATE_SQLITE = ".rove-provider-integration-state/state.sqlite"
$env:ROVE_MEMORY_SESSION_DIR = ".rove-provider-integration-state/memory/sessions"
$env:ROVE_MEMORY_DURABLE_DIR = ".rove-provider-integration-state/memory"

@{
    provider = $Provider
    model = $Model
    api_base = $ApiBase
    key_env = $ApiKeyEnv
    key_present = [bool](Get-ApiKeyValue)
    models_endpoint = Get-DefaultModelsEndpoint
    api_base_local = $ApiBaseLocal
    web_base = $WebBase
    integration_root = $IntegrationRoot
} | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "environment.redacted.json")

$gates = @{}
try {
    $gates["model_inventory"] = Invoke-ModelInventory
    $gates["provider_smoke"] = Invoke-ProviderSmoke
    $gates["provider_full_api"] = Invoke-ApiSmoke
    $gates["web_provider"] = Invoke-WebSmoke
    $gates["stress"] = Invoke-StressGate
    $gates["external_mcp"] = Invoke-ExternalMcpGate
    Write-EvidenceSummary $gates
    Write-Host "provider integration completed"
    Write-Host "Artifacts: $ArtifactsDir"
    Write-Host "Model: $Model"
    Write-Host "Provider: $Provider"
} catch {
    $gates["failure"] = $_.Exception.Message
    Write-EvidenceSummary $gates
    throw
}
