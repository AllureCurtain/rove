param(
    [string]$Provider = $(if ($env:ROVE_PROVIDER_INTEGRATION_PROVIDER) { $env:ROVE_PROVIDER_INTEGRATION_PROVIDER } else { "openai-compatible" }),
    [string]$Model = $(if ($env:ROVE_PROVIDER_INTEGRATION_MODEL) { $env:ROVE_PROVIDER_INTEGRATION_MODEL } elseif ($env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL) { $env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL } else { "" }),
    [string]$ApiBase = $(if ($env:ROVE_PROVIDER_INTEGRATION_API_BASE) { $env:ROVE_PROVIDER_INTEGRATION_API_BASE } else { "" }),
    [string]$ApiKeyEnv = $(if ($env:ROVE_PROVIDER_INTEGRATION_API_KEY_ENV) { $env:ROVE_PROVIDER_INTEGRATION_API_KEY_ENV } else { "" }),
    [string]$ModelsEndpoint = $(if ($env:ROVE_PROVIDER_INTEGRATION_MODELS_ENDPOINT) { $env:ROVE_PROVIDER_INTEGRATION_MODELS_ENDPOINT } else { "" }),
    [string]$IntegrationRoot = $(if ($env:ROVE_PROVIDER_INTEGRATION_ROOT) { $env:ROVE_PROVIDER_INTEGRATION_ROOT } else { (Join-Path $env:TEMP ("rove-provider-integration-" + [guid]::NewGuid().ToString("N"))) }),
    [string]$ApiAddr = $(if ($env:ROVE_PROVIDER_INTEGRATION_API_ADDR) { $env:ROVE_PROVIDER_INTEGRATION_API_ADDR } else { "127.0.0.1:18791" }),
    [string]$WebPort = $(if ($env:ROVE_PROVIDER_INTEGRATION_WEB_PORT) { $env:ROVE_PROVIDER_INTEGRATION_WEB_PORT } else { "13021" }),
    [switch]$SkipModelInventory,
    [switch]$SkipProviderSmoke,
    [switch]$SkipApiSmoke,
    [switch]$SkipWebSmoke,
    [switch]$RunStress,
    [switch]$RunRestartRecovery,
    [switch]$RunLongSoak,
    [switch]$RunExternalMcp,
    [int]$StressSequentialCount = $(if ($env:ROVE_PROVIDER_INTEGRATION_STRESS_SEQUENTIAL_COUNT) { [int]$env:ROVE_PROVIDER_INTEGRATION_STRESS_SEQUENTIAL_COUNT } else { 5 }),
    [int]$StressConcurrentCount = $(if ($env:ROVE_PROVIDER_INTEGRATION_STRESS_CONCURRENT_COUNT) { [int]$env:ROVE_PROVIDER_INTEGRATION_STRESS_CONCURRENT_COUNT } else { 3 }),
    [int]$StressJobTimeoutSeconds = $(if ($env:ROVE_PROVIDER_INTEGRATION_STRESS_JOB_TIMEOUT_SECONDS) { [int]$env:ROVE_PROVIDER_INTEGRATION_STRESS_JOB_TIMEOUT_SECONDS } else { 180 }),
    [int]$RestartRecoveryTimeoutSeconds = $(if ($env:ROVE_PROVIDER_INTEGRATION_RESTART_TIMEOUT_SECONDS) { [int]$env:ROVE_PROVIDER_INTEGRATION_RESTART_TIMEOUT_SECONDS } else { 90 }),
    [int]$LongSoakCount = $(if ($env:ROVE_PROVIDER_INTEGRATION_LONG_SOAK_COUNT) { [int]$env:ROVE_PROVIDER_INTEGRATION_LONG_SOAK_COUNT } else { 20 }),
    [int]$LongSoakDelayMs = $(if ($env:ROVE_PROVIDER_INTEGRATION_LONG_SOAK_DELAY_MS) { [int]$env:ROVE_PROVIDER_INTEGRATION_LONG_SOAK_DELAY_MS } else { 500 }),
    [string]$ExternalMcpToolName = $(if ($env:ROVE_PROVIDER_INTEGRATION_EXTERNAL_MCP_TOOL) { $env:ROVE_PROVIDER_INTEGRATION_EXTERNAL_MCP_TOOL } else { "mcp__mock_server__echo_remote" }),
    [switch]$KeepState
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$script:StressApiProcess = $null
$script:CurrentGateName = ""

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

function Use-TruthyEnvironmentValue([string]$Value) {
    return $Value -match "^(1|true|yes|on)$"
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

function Add-CorsOrigins([string[]]$Origins) {
    $existing = @()
    if ($env:ROVE_API_CORS_ORIGINS) {
        $existing = $env:ROVE_API_CORS_ORIGINS.Split(",") | ForEach-Object { $_.Trim() } | Where-Object { $_ }
    }

    $merged = [System.Collections.Generic.List[string]]::new()
    foreach ($origin in @($existing + $Origins)) {
        if ($origin -and -not $merged.Contains($origin)) {
            [void]$merged.Add($origin)
        }
    }
    $env:ROVE_API_CORS_ORIGINS = $merged -join ","
}

function Normalize-ProviderName([string]$Name) {
    $normalized = $Name.Trim().ToLowerInvariant()
    switch ($normalized) {
        "openai" { return "openai-compatible" }
        "openai-compatible" { return "openai-compatible" }
        "openai-responses" { return "openai-responses" }
        "responses" { return "openai-responses" }
        "anthropic" { return "anthropic" }
        "ollama" { return "ollama" }
        default {
            throw "Unsupported provider '$Name'. Expected openai-compatible, openai-responses, anthropic, or ollama."
        }
    }
}

function Provider-RequiresKey([string]$Name) {
    $normalized = Normalize-ProviderName $Name
    return $normalized -in @("openai-compatible", "openai-responses", "anthropic")
}

function Default-KeyEnvForProvider([string]$Name) {
    $normalized = Normalize-ProviderName $Name
    if ($normalized -eq "ollama") {
        return ""
    }
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

function Wait-JobTerminal([string]$JobId, [string]$Name, [int]$TimeoutSeconds = 180, [string]$StateArtifactName = "") {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $lastState = $null
    while ((Get-Date) -lt $deadline) {
        $lastState = Invoke-Json -Method Get -Uri "$ApiBaseLocal/jobs/$JobId/state"
        if ($lastState.status -in @("done", "error", "cancelled", "interrupted")) {
            $artifactName = if ($StateArtifactName) { $StateArtifactName } else { "provider-full-$Name.state.json" }
            $lastState | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir $artifactName)
            return $lastState
        }
        Start-Sleep -Milliseconds 750
    }
    $snapshot = if ($null -ne $lastState) { $lastState | ConvertTo-Json -Depth 20 -Compress } else { "<no state>" }
    throw "Timed out waiting for $Name job $JobId. Last state: $snapshot"
}

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

function Test-ProviderKeyPresent {
    try {
        return [bool](Get-ApiKeyValue)
    } catch {
        return $false
    }
}

function Set-ProviderEnvironment {
    $key = Get-ApiKeyValue
    $env:ROVE_PROVIDER = $Provider
    $env:ROVE_MODEL = $Model

    if ($Provider -in @("openai-compatible", "openai-responses")) {
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

function Invoke-ProviderRestMethod([string]$Uri, [hashtable]$Headers = $null) {
    try {
        if ($Headers) {
            return Invoke-RestMethod -Uri $Uri -Headers $Headers -TimeoutSec 30
        }
        return Invoke-RestMethod -Uri $Uri -TimeoutSec 30
    } catch {
        throw "Provider request to $Uri failed: $($_.Exception.Message)"
    }
}

function Invoke-OpenAiCompatibleModelInventory {
    $endpoint = Get-DefaultModelsEndpoint
    $headers = @{ Authorization = "Bearer $(Get-ApiKeyValue)" }
    $models = Invoke-ProviderRestMethod -Uri $endpoint -Headers $headers
    $items = @($models.data)
    $visible = $items | Where-Object { $_.id } | Sort-Object id
    $visible | Select-Object id, owned_by | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "provider-models.json")
    return $visible.id
}

function Invoke-AnthropicModelInventory {
    $endpoint = Get-DefaultModelsEndpoint
    $headers = @{
        "x-api-key" = Get-ApiKeyValue
        "anthropic-version" = "2023-06-01"
    }
    $models = Invoke-ProviderRestMethod -Uri $endpoint -Headers $headers
    $items = @($models.data)
    $visible = $items | Where-Object { $_.id } | Sort-Object id
    $visible | Select-Object id, display_name, created_at | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "provider-models.json")
    return $visible.id
}

function Invoke-OllamaModelInventory {
    $endpoint = Get-DefaultModelsEndpoint
    $models = Invoke-ProviderRestMethod -Uri $endpoint
    $items = @($models.models)
    $visible = $items | Where-Object { $_.name } | Sort-Object name
    $visible | Select-Object name, model, modified_at, size | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "provider-models.json")
    return $visible.name
}

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

function Invoke-ProviderSmoke {
    if ($SkipProviderSmoke) {
        return "skipped"
    }
    $env:ROVE_PROVIDER_SMOKE_OPENAI = "0"
    $env:ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES = "0"
    $env:ROVE_PROVIDER_SMOKE_ANTHROPIC = "0"
    $env:ROVE_PROVIDER_SMOKE_OLLAMA = "0"

    $testName = ""
    switch ($Provider) {
        "openai-compatible" {
            $env:ROVE_PROVIDER_SMOKE_OPENAI = "1"
            $env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL = $Model
            $testName = "openai_compatible_real_provider_smoke_when_enabled"
        }
        "openai-responses" {
            $env:ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES = "1"
            $env:ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES_MODEL = $Model
            $testName = "openai_responses_real_provider_smoke_when_enabled"
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

function New-ProviderProfileBody {
    if (Provider-RequiresKey $Provider) {
        $provider = @{
            name = $Provider
            api_base = $ApiBase
            api_key_env = $ApiKeyEnv
        }
        return $provider
    }
    $provider = @{
        name = $Provider
        api_base = $ApiBase
    }
    return $provider
}

function Classify-ProviderOutput([int]$ExitCode, [string]$Text) {
    if ($ExitCode -eq 0) {
        return "pass"
    }
    if ($Text -match "\b(401|403)\b|Unauthorized|Invalid token|invalid api key|api key is not set|authentication|permission denied") {
        return "key/configuration"
    }
    if ($Text -match "429|rate limit|quota|too many requests") {
        return "quota/rate limit"
    }
    if ($Text -match "Provider request to .* failed|request failed|error sending request|Connect|timed out|timeout|connection|dns|NameResolution|refused|unreachable") {
        return "network/connectivity"
    }
    if ($Text -match "did not emit an echo tool call|did not complete echo tool output|unexpected provider smoke output|tool-use|tool use") {
        return "model tool-use/follow-up behavior"
    }
    if ($Text -match "panic|SQLite|database is locked|lost run_id|corrupt|missing report") {
        return "rove runtime defect"
    }
    return "rove runtime or assertion failure"
}

function Classify-RunReport([object]$Report, [object]$State) {
    $text = (($Report.output, ($Report | ConvertTo-Json -Depth 20 -Compress), ($State | ConvertTo-Json -Depth 20 -Compress)) -join "`n")
    if ($State.status -eq "done" -and $Report.status -eq "success") {
        return "pass"
    }
    return Classify-ProviderOutput -ExitCode 1 -Text $text
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
            provider = New-ProviderProfileBody
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
            provider = New-ProviderProfileBody
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
        Add-CorsOrigins @($WebBase, "http://localhost:$WebPort")
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
        $env:ROVE_WEB_PROVIDER = $Provider
        $env:ROVE_WEB_PROVIDER_API_BASE = $ApiBase
        $env:ROVE_WEB_PROVIDER_KEY_ENV = $ApiKeyEnv
        $nodeScript = @'
const { chromium } = require('@playwright/test');
const fs = require('fs');
const baseURL = process.env.PLAYWRIGHT_BASE_URL;
const model = process.env.ROVE_MODEL;
const provider = process.env.ROVE_WEB_PROVIDER;
const providerApiBase = process.env.ROVE_WEB_PROVIDER_API_BASE;
const providerKeyEnv = process.env.ROVE_WEB_PROVIDER_KEY_ENV;
const resultPath = process.env.ROVE_WEB_PROVIDER_RESULT;
const screenshotPath = process.env.ROVE_WEB_PROVIDER_SCREENSHOT;
const prompt = 'Use the echo tool exactly once with message "rove provider web ok". After the tool returns, reply only with that exact message.';
(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  await page.goto(baseURL, { waitUntil: 'networkidle' });
  if (provider && provider !== 'default') {
    await page.getByLabel('Provider').selectOption(provider);
    await page.getByLabel('API base').fill(providerApiBase);
    const keyEnv = page.getByLabel('Key env');
    if (await keyEnv.count()) {
      await keyEnv.fill(providerKeyEnv);
    }
  }
  await page.getByLabel('Task').fill(prompt);
  await page.getByLabel('Model').fill(model);
  await page.getByLabel('Steps').fill('4');
  await page.getByRole('button', { name: 'Run' }).click();
  await page.getByLabel('Run summary').getByText('Run completed').first().waitFor({ timeout: 120000 });
  await page.screenshot({ path: screenshotPath, fullPage: true });
  const result = {
    baseURL,
    model,
    provider,
    providerApiBase,
    providerKeyEnv: providerKeyEnv || '',
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
            @{
                classification = Classify-ProviderOutput -ExitCode 1 -Text "lost run_id $($job.run_id)"
                lost_run_id = [string]$job.run_id
                before_count = @($before.runs).Count
                after_count = @($after.runs).Count
            } | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "restart-recovery-failure.json")
            throw "restart recovery lost run_id $($job.run_id)"
        }
    }
    return "pass"
}

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
        $state = Wait-JobTerminal -JobId $job.job_id -Name "long-soak-$i" -TimeoutSeconds $StressJobTimeoutSeconds -StateArtifactName "long-soak-$i.state.json"
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
            $classification = Classify-RunReport -Report $report -State $state
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

function Write-StressSummary(
    [array]$CreatedJobs,
    [string]$RestartStatus,
    [string]$LongSoakStatus,
    [string]$FailedGate = "",
    [int]$FailedIndex = 0,
    [string]$Classification = ""
) {
    $summary = @{
        sequential_count = $StressSequentialCount
        concurrent_count = $StressConcurrentCount
        restart_recovery = $RestartStatus
        long_soak = $LongSoakStatus
        jobs = $CreatedJobs
    }
    if ($FailedGate) {
        $summary.failed_gate = $FailedGate
        $summary.failed_index = $FailedIndex
        $summary.classification = $Classification
    }
    if ($FailedGate -eq "long_soak") {
        $summary.long_soak_summary = "long-soak-summary.json"
    }
    if ($RestartStatus -ne "not_run") {
        $runs = Invoke-Json -Method Get -Uri "$ApiBaseLocal/runs?limit=100"
        $summary.runs_count = @($runs.runs).Count
        $runs | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "stress-runs.json")
    }
    $summary | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "stress-summary.json")
}

function Invoke-StressGate {
    if (-not $RunStress) {
        return "skipped"
    }
    try {
        $stressWorkspace = Join-Path $IntegrationRoot "workspace-stress"
        New-Item -ItemType Directory -Force -Path $stressWorkspace | Out-Null
        $script:StressApiProcess = Start-BackgroundCommand -Command "cargo" -Arguments @("run", "--bin", "rove-api", "--", "--addr", $ApiAddr, "-C", $stressWorkspace) -WorkingDirectory $RepoRoot -StdoutLog (Join-Path $ArtifactsDir "stress-api.out.log") -StderrLog (Join-Path $ArtifactsDir "stress-api.err.log")
        Wait-HttpOk -Uri "$ApiBaseLocal/runs?limit=1" -TimeoutSeconds 90 -Name "stress rove-api"

        $created = @()
        for ($i = 1; $i -le $StressSequentialCount; $i++) {
            $job = Invoke-Json -Method Post -Uri "$ApiBaseLocal/jobs" -Body @{
                message = "Reply with exactly: rove provider stress ok $i"
                model = $Model
                approval = "auto"
                max_steps = 4
                provider = New-ProviderProfileBody
            }
            $state = Wait-JobTerminal -JobId $job.job_id -Name "stress-sequential-$i" -TimeoutSeconds $StressJobTimeoutSeconds -StateArtifactName "stress-sequential-$i.state.json"
            $report = Invoke-Json -Method Get -Uri "$ApiBaseLocal/runs/$($job.run_id)/report"
            $report | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "stress-sequential-$i.report.json")
            $classification = Classify-RunReport -Report $report -State $state
            $record = @{
                kind = "sequential"
                index = $i
                job_id = [string]$job.job_id
                run_id = [string]$job.run_id
                status = [string]$state.status
                report_status = [string]$report.status
                classification = $classification
            }
            $created += $record
            if ($classification -ne "pass") {
                Write-StressSummary -CreatedJobs $created -RestartStatus "not_run" -LongSoakStatus "not_run" -FailedGate "sequential" -FailedIndex $i -Classification $classification
                throw "sequential stress job $i ended with classification '$classification'"
            }
        }

        $jobs = @()
        for ($i = 1; $i -le $StressConcurrentCount; $i++) {
            $jobs += Invoke-Json -Method Post -Uri "$ApiBaseLocal/jobs" -Body @{
                message = "Reply with exactly: rove provider concurrent stress ok $i"
                model = $Model
                approval = "auto"
                max_steps = 4
                provider = New-ProviderProfileBody
            }
        }
        for ($i = 0; $i -lt $jobs.Count; $i++) {
            $state = Wait-JobTerminal -JobId $jobs[$i].job_id -Name "stress-concurrent-$($i + 1)" -TimeoutSeconds $StressJobTimeoutSeconds -StateArtifactName "stress-concurrent-$($i + 1).state.json"
            $report = Invoke-Json -Method Get -Uri "$ApiBaseLocal/runs/$($jobs[$i].run_id)/report"
            $report | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "stress-concurrent-$($i + 1).report.json")
            $classification = Classify-RunReport -Report $report -State $state
            $record = @{
                kind = "concurrent"
                index = $i + 1
                job_id = [string]$jobs[$i].job_id
                run_id = [string]$jobs[$i].run_id
                status = [string]$state.status
                report_status = [string]$report.status
                classification = $classification
            }
            $created += $record
            if ($classification -ne "pass") {
                Write-StressSummary -CreatedJobs $created -RestartStatus "not_run" -LongSoakStatus "not_run" -FailedGate "concurrent" -FailedIndex ($i + 1) -Classification $classification
                throw "concurrent stress job $($i + 1) ended with classification '$classification'"
            }
        }

        $restartStatus = Invoke-RestartRecoveryGate -CreatedJobs $created -StressWorkspace $stressWorkspace
        try {
            $longSoakStatus = Invoke-LongSoakGate
        } catch {
            $longSoakSummaryPath = Join-Path $ArtifactsDir "long-soak-summary.json"
            $classification = Classify-ProviderOutput -ExitCode 1 -Text $_.Exception.Message
            $failedIndex = 0
            if (Test-Path -LiteralPath $longSoakSummaryPath) {
                $longSoakSummary = Get-Content -Raw -LiteralPath $longSoakSummaryPath | ConvertFrom-Json
                $classification = [string]$longSoakSummary.classification
                $failedIndex = [int]$longSoakSummary.failed_index
            }
            Write-StressSummary -CreatedJobs $created -RestartStatus $restartStatus -LongSoakStatus "failed" -FailedGate "long_soak" -FailedIndex $failedIndex -Classification $classification
            throw
        }
        Write-StressSummary -CreatedJobs $created -RestartStatus $restartStatus -LongSoakStatus $longSoakStatus
        return "pass"
    } finally {
        if ($script:StressApiProcess) {
            Stop-ProcessTree $script:StressApiProcess
            $script:StressApiProcess = $null
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
        key_present = Test-ProviderKeyPresent
        gates = $Gates
    } | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "evidence-summary.json")
}

function Read-GateClassification([string]$GateName) {
    $path = switch ($GateName) {
        "provider_smoke" { Join-Path $ArtifactsDir "provider-smoke-result.json" }
        default { "" }
    }
    if (-not $path -or -not (Test-Path -LiteralPath $path)) {
        return ""
    }
    try {
        $result = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
        if ($result.classification) {
            return [string]$result.classification
        }
    } catch {
        return ""
    }
    return ""
}

function New-RequestedGateStatusMap {
    return @{
        model_inventory = if ($SkipModelInventory) { "skipped" } else { "not_run" }
        provider_smoke = if ($SkipProviderSmoke) { "skipped" } else { "not_run" }
        provider_full_api = if ($SkipApiSmoke) { "skipped" } else { "not_run" }
        web_provider = if ($SkipWebSmoke) { "skipped" } else { "not_run" }
        stress = if ($RunStress) { "not_run" } else { "skipped" }
        external_mcp = if ($RunExternalMcp) { "not_run" } else { "skipped" }
    }
}

function Invoke-Gate([hashtable]$Gates, [string]$GateName, [scriptblock]$Action) {
    $script:CurrentGateName = $GateName
    try {
        $Gates[$GateName] = & $Action
        $script:CurrentGateName = ""
        return $Gates[$GateName]
    } catch {
        $Gates[$GateName] = "failed"
        throw
    }
}

function Write-GateFailureArtifact([hashtable]$Gates, [string]$Message) {
    if (-not $script:CurrentGateName) {
        $script:CurrentGateName = "runner_setup"
    }
    if ($Gates.ContainsKey($script:CurrentGateName)) {
        $Gates[$script:CurrentGateName] = "failed"
    }
    $classification = Read-GateClassification -GateName $script:CurrentGateName
    if (-not $classification) {
        $classification = Classify-ProviderOutput -ExitCode 1 -Text $Message
    }
    @{
        date = (Get-Date).ToString("o")
        failed_gate = $script:CurrentGateName
        classification = $classification
        message = $Message
        provider = $Provider
        api_base = $ApiBase
        model = $Model
        artifact_dir = $ArtifactsDir
    } | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "failure-classification.json")
}

$RepoRoot = Split-Path -Parent $PSScriptRoot
Import-DotEnvNoOverride (Join-Path $RepoRoot ".env.integration")

if (-not $PSBoundParameters.ContainsKey("Provider") -and $env:ROVE_PROVIDER_INTEGRATION_PROVIDER) {
    $Provider = $env:ROVE_PROVIDER_INTEGRATION_PROVIDER
}
if (-not $PSBoundParameters.ContainsKey("Model") -and $env:ROVE_PROVIDER_INTEGRATION_MODEL) {
    $Model = $env:ROVE_PROVIDER_INTEGRATION_MODEL
}
if (-not $PSBoundParameters.ContainsKey("Model") -and -not $Model -and $env:ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES_MODEL) {
    $Model = $env:ROVE_PROVIDER_SMOKE_OPENAI_RESPONSES_MODEL
}
if (-not $PSBoundParameters.ContainsKey("Model") -and -not $Model -and $env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL) {
    $Model = $env:ROVE_PROVIDER_SMOKE_OPENAI_MODEL
}
if (-not $PSBoundParameters.ContainsKey("ApiBase") -and $env:ROVE_PROVIDER_INTEGRATION_API_BASE) {
    $ApiBase = $env:ROVE_PROVIDER_INTEGRATION_API_BASE
}
if (-not $PSBoundParameters.ContainsKey("ApiKeyEnv") -and $env:ROVE_PROVIDER_INTEGRATION_API_KEY_ENV) {
    $ApiKeyEnv = $env:ROVE_PROVIDER_INTEGRATION_API_KEY_ENV
}
if (-not $PSBoundParameters.ContainsKey("ModelsEndpoint") -and $env:ROVE_PROVIDER_INTEGRATION_MODELS_ENDPOINT) {
    $ModelsEndpoint = $env:ROVE_PROVIDER_INTEGRATION_MODELS_ENDPOINT
}
if (-not $PSBoundParameters.ContainsKey("IntegrationRoot") -and $env:ROVE_PROVIDER_INTEGRATION_ROOT) {
    $IntegrationRoot = $env:ROVE_PROVIDER_INTEGRATION_ROOT
}
if (-not $PSBoundParameters.ContainsKey("ApiAddr") -and $env:ROVE_PROVIDER_INTEGRATION_API_ADDR) {
    $ApiAddr = $env:ROVE_PROVIDER_INTEGRATION_API_ADDR
}
if (-not $PSBoundParameters.ContainsKey("WebPort") -and $env:ROVE_PROVIDER_INTEGRATION_WEB_PORT) {
    $WebPort = $env:ROVE_PROVIDER_INTEGRATION_WEB_PORT
}
if (-not $PSBoundParameters.ContainsKey("StressSequentialCount") -and $env:ROVE_PROVIDER_INTEGRATION_STRESS_SEQUENTIAL_COUNT) {
    $StressSequentialCount = [int]$env:ROVE_PROVIDER_INTEGRATION_STRESS_SEQUENTIAL_COUNT
}
if (-not $PSBoundParameters.ContainsKey("StressConcurrentCount") -and $env:ROVE_PROVIDER_INTEGRATION_STRESS_CONCURRENT_COUNT) {
    $StressConcurrentCount = [int]$env:ROVE_PROVIDER_INTEGRATION_STRESS_CONCURRENT_COUNT
}
if (-not $PSBoundParameters.ContainsKey("StressJobTimeoutSeconds") -and $env:ROVE_PROVIDER_INTEGRATION_STRESS_JOB_TIMEOUT_SECONDS) {
    $StressJobTimeoutSeconds = [int]$env:ROVE_PROVIDER_INTEGRATION_STRESS_JOB_TIMEOUT_SECONDS
}
if (-not $PSBoundParameters.ContainsKey("RestartRecoveryTimeoutSeconds") -and $env:ROVE_PROVIDER_INTEGRATION_RESTART_TIMEOUT_SECONDS) {
    $RestartRecoveryTimeoutSeconds = [int]$env:ROVE_PROVIDER_INTEGRATION_RESTART_TIMEOUT_SECONDS
}
if (-not $PSBoundParameters.ContainsKey("LongSoakCount") -and $env:ROVE_PROVIDER_INTEGRATION_LONG_SOAK_COUNT) {
    $LongSoakCount = [int]$env:ROVE_PROVIDER_INTEGRATION_LONG_SOAK_COUNT
}
if (-not $PSBoundParameters.ContainsKey("LongSoakDelayMs") -and $env:ROVE_PROVIDER_INTEGRATION_LONG_SOAK_DELAY_MS) {
    $LongSoakDelayMs = [int]$env:ROVE_PROVIDER_INTEGRATION_LONG_SOAK_DELAY_MS
}
if (-not $PSBoundParameters.ContainsKey("ExternalMcpToolName") -and $env:ROVE_PROVIDER_INTEGRATION_EXTERNAL_MCP_TOOL) {
    $ExternalMcpToolName = $env:ROVE_PROVIDER_INTEGRATION_EXTERNAL_MCP_TOOL
}
$Provider = Normalize-ProviderName $Provider
if (-not $PSBoundParameters.ContainsKey("ApiBase") -and -not $ApiBase -and ($Provider -in @("openai-compatible", "openai-responses")) -and $env:OPENAI_API_BASE) {
    $ApiBase = $env:OPENAI_API_BASE
}
if ($Provider -eq "ollama") {
    $ApiKeyEnv = ""
} elseif (-not $PSBoundParameters.ContainsKey("ApiKeyEnv") -and $Provider -eq "anthropic" -and (-not $ApiKeyEnv -or $ApiKeyEnv -eq "OPENAI_API_KEY")) {
    $ApiKeyEnv = Default-KeyEnvForProvider $Provider
} elseif (-not $ApiKeyEnv) {
    $ApiKeyEnv = Default-KeyEnvForProvider $Provider
}
$ApiBase = Default-ApiBaseForProvider $Provider $ApiBase
if (-not $PSBoundParameters.ContainsKey("RunRestartRecovery") -and (Use-TruthyEnvironmentValue $env:ROVE_PROVIDER_INTEGRATION_RUN_RESTART_RECOVERY)) {
    $RunRestartRecovery = $true
}
if (-not $PSBoundParameters.ContainsKey("RunLongSoak") -and (Use-TruthyEnvironmentValue $env:ROVE_PROVIDER_INTEGRATION_RUN_LONG_SOAK)) {
    $RunLongSoak = $true
}

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

$gates = New-RequestedGateStatusMap
try {
    @{
        provider = $Provider
        model = $Model
        api_base = $ApiBase
        key_env = $ApiKeyEnv
        key_present = Test-ProviderKeyPresent
        models_endpoint = Get-DefaultModelsEndpoint
        api_base_local = $ApiBaseLocal
        web_base = $WebBase
        integration_root = $IntegrationRoot
    } | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $ArtifactsDir "environment.redacted.json")

    Test-CommandAvailable "cargo"
    Test-CommandAvailable "pnpm"
    Set-ProviderEnvironment
    $env:ROVE_STATE_DIR = ".rove-provider-integration-state"
    $env:ROVE_STATE_SQLITE = ".rove-provider-integration-state/state.sqlite"
    $env:ROVE_MEMORY_SESSION_DIR = ".rove-provider-integration-state/memory/sessions"
    $env:ROVE_MEMORY_DURABLE_DIR = ".rove-provider-integration-state/memory"

    Invoke-Gate -Gates $gates -GateName "model_inventory" -Action { Invoke-ModelInventory } | Out-Null
    Invoke-Gate -Gates $gates -GateName "provider_smoke" -Action { Invoke-ProviderSmoke } | Out-Null
    Invoke-Gate -Gates $gates -GateName "provider_full_api" -Action { Invoke-ApiSmoke } | Out-Null
    Invoke-Gate -Gates $gates -GateName "web_provider" -Action { Invoke-WebSmoke } | Out-Null
    Invoke-Gate -Gates $gates -GateName "stress" -Action { Invoke-StressGate } | Out-Null
    Invoke-Gate -Gates $gates -GateName "external_mcp" -Action { Invoke-ExternalMcpGate } | Out-Null
    Write-EvidenceSummary $gates
    Write-Host "provider integration completed"
    Write-Host "Artifacts: $ArtifactsDir"
    Write-Host "Model: $Model"
    Write-Host "Provider: $Provider"
} catch {
    Write-GateFailureArtifact -Gates $gates -Message $_.Exception.Message
    Write-EvidenceSummary $gates
    throw
}
