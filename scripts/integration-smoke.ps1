param(
    [string]$Profile = $(if ($env:ROVE_INTEGRATION_PROFILE) { $env:ROVE_INTEGRATION_PROFILE } else { "local-full" }),
    [string]$ApiAddr = $(if ($env:ROVE_API_BIND_ADDR) { $env:ROVE_API_BIND_ADDR } else { "127.0.0.1:8787" }),
    [string]$WebPort = $(if ($env:ROVE_WEB_PORT) { $env:ROVE_WEB_PORT } else { "3000" }),
    [string]$IntegrationRoot = $(if ($env:ROVE_INTEGRATION_ROOT) { $env:ROVE_INTEGRATION_ROOT } else { (Join-Path $env:TEMP "rove-integration") }),
    [switch]$SkipApiSmoke,
    [switch]$SkipWebE2E,
    [switch]$KeepState
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-RepoPath([string]$Path) {
    if ([System.IO.Path]::IsPathRooted($Path)) {
        return $Path
    }
    return (Join-Path $RepoRoot $Path)
}

function Import-DotEnv([string]$Path) {
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
        [Environment]::SetEnvironmentVariable($name, $value, "Process")
    }
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

function Wait-HttpOk([string]$Uri, [int]$TimeoutSeconds, [string]$Name) {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $lastError = $null
    while ((Get-Date) -lt $deadline) {
        try {
            $headers = @{}
            if ($env:ROVE_API_TOKEN -and $Uri.StartsWith("$ApiBase/")) {
                $headers["Authorization"] = "Bearer $env:ROVE_API_TOKEN"
            }
            $response = Invoke-WebRequest -UseBasicParsing -Uri $Uri -Headers $headers -TimeoutSec 3
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
    $headers = @{}
    if ($env:ROVE_API_TOKEN) {
        $headers["Authorization"] = "Bearer $env:ROVE_API_TOKEN"
    }
    if ($null -eq $Body) {
        return Invoke-RestMethod -Method $Method -Uri $Uri -Headers $headers
    }
    $json = $Body | ConvertTo-Json -Depth 20 -Compress
    return Invoke-RestMethod -Method $Method -Uri $Uri -Headers $headers -ContentType "application/json" -Body $json
}

function Wait-JobState([string]$JobId, [scriptblock]$Predicate, [string]$Description, [int]$TimeoutSeconds = 30) {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $lastState = $null
    while ((Get-Date) -lt $deadline) {
        $lastState = Invoke-Json -Method Get -Uri "$ApiBase/jobs/$JobId/state"
        if (& $Predicate $lastState) {
            return $lastState
        }
        Start-Sleep -Milliseconds 500
    }
    $snapshot = if ($null -ne $lastState) { $lastState | ConvertTo-Json -Depth 20 -Compress } else { "<no state>" }
    throw "Timed out waiting for job ${JobId}: $Description. Last state: $snapshot"
}

function Start-JobRequest([string]$Name, [hashtable]$Body) {
    $created = Invoke-Json -Method Post -Uri "$ApiBase/jobs" -Body $Body
    $script:RunIds += [string]$created.run_id
    $created | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $ApiArtifacts "$Name.created.json")
    return $created
}

function Save-State([string]$Name, [object]$State) {
    $State | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath (Join-Path $ApiArtifacts "$Name.state.json")
}

function Assert-TerminalDone([string]$Name, [object]$Created) {
    $state = Wait-JobState -JobId $Created.job_id -Description "$Name to finish" -Predicate {
        param($s)
        $s.status -eq "done"
    }
    Save-State -Name $Name -State $state
    return $state
}

function Run-ApiSmoke {
    New-Item -ItemType Directory -Force -Path $ApiArtifacts | Out-Null

    $plain = Start-JobRequest "plain" @{
        message = "local-full plain run"
        model = "fake"
        approval = "auto"
        max_steps = 4
    }
    Assert-TerminalDone "plain" $plain | Out-Null

    $echo = Start-JobRequest "echo" @{
        message = '{"tool":"echo","args":{"message":"hello local-full"}}'
        model = "fake-raw"
        approval = "auto"
        max_steps = 1
    }
    $echoState = Assert-TerminalDone "echo" $echo
    if (-not (($echoState.events | ConvertTo-Json -Depth 30) -match "hello local-full")) {
        throw "Echo smoke did not record expected tool output."
    }

    $approvedPath = "approved-api-smoke.txt"
    $approval = Start-JobRequest "approval-approved" @{
        message = '{"tool":"write_file","args":{"path":"approved-api-smoke.txt","content":"ok"}}'
        model = "fake-raw"
        approval = "ask"
        max_steps = 1
    }
    $pendingApproval = Wait-JobState -JobId $approval.job_id -Description "pending approval" -Predicate {
        param($s)
        $s.pending_approvals.Count -gt 0
    }
    $callId = $pendingApproval.pending_approvals[0].call_id
    Invoke-Json -Method Post -Uri "$ApiBase/jobs/$($approval.job_id)/approvals/$callId" -Body @{ decision = "approve" } | Out-Null
    Assert-TerminalDone "approval-approved" $approval | Out-Null
    $approvedFullPath = Join-Path $WorkspaceDir $approvedPath
    if (-not (Test-Path -LiteralPath $approvedFullPath)) {
        throw "Approved fs_write did not create $approvedFullPath."
    }

    $rejected = Start-JobRequest "approval-rejected" @{
        message = '{"tool":"write_file","args":{"path":"rejected-api-smoke.txt","content":"no"}}'
        model = "fake-raw"
        approval = "ask"
        max_steps = 1
    }
    $pendingReject = Wait-JobState -JobId $rejected.job_id -Description "pending rejection approval" -Predicate {
        param($s)
        $s.pending_approvals.Count -gt 0
    }
    $rejectCallId = $pendingReject.pending_approvals[0].call_id
    Invoke-Json -Method Post -Uri "$ApiBase/jobs/$($rejected.job_id)/approvals/$rejectCallId" -Body @{ decision = "reject" } | Out-Null
    $rejectedState = Wait-JobState -JobId $rejected.job_id -Description "rejected tool record" -Predicate {
        param($s)
        ($s.events | ConvertTo-Json -Depth 30) -match "tool_call_failed"
    }
    Save-State -Name "approval-rejected" -State $rejectedState
    if (Test-Path -LiteralPath (Join-Path $WorkspaceDir "rejected-api-smoke.txt")) {
        throw "Rejected fs_write unexpectedly created rejected-api-smoke.txt."
    }

    $input = Start-JobRequest "input" @{
        message = '{"tool":"request_input","args":{"prompt":"Which branch should I use?"}}'
        model = "fake-raw"
        approval = "auto"
        max_steps = 1
    }
    $pendingInput = Wait-JobState -JobId $input.job_id -Description "pending input" -Predicate {
        param($s)
        $s.pending_inputs.Count -gt 0
    }
    $inputId = $pendingInput.pending_inputs[0].input_id
    Invoke-Json -Method Post -Uri "$ApiBase/jobs/$($input.job_id)/inputs/$inputId" -Body @{ answer = "main" } | Out-Null
    $inputState = Assert-TerminalDone "input" $input
    if (-not (($inputState.events | ConvertTo-Json -Depth 30) -match "main")) {
        throw "request_input smoke did not record supplied answer."
    }

    $failure = Start-JobRequest "failure" @{
        message = '{"tool":"read_file","args":{"path":"missing-api-smoke.txt"}}'
        model = "fake-raw"
        approval = "auto"
        max_steps = 1
    }
    $failureState = Wait-JobState -JobId $failure.job_id -Description "tool failure event" -Predicate {
        param($s)
        ($s.events | ConvertTo-Json -Depth 30) -match "tool_call_failed"
    }
    Save-State -Name "failure" -State $failureState

    $runs = Invoke-Json -Method Get -Uri "$ApiBase/runs?limit=25"
    $runs | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $ApiArtifacts "runs.json")
    foreach ($runId in $script:RunIds) {
        if (-not ($runs.runs | Where-Object { $_.run_id -eq $runId })) {
            throw "Run $runId was not present in /runs history."
        }
    }
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

if ($Profile -ne "local-full") {
    throw "This runner currently implements only the local-full profile. Received: $Profile"
}

$RepoRoot = Split-Path -Parent $PSScriptRoot
Import-DotEnv (Join-Path $RepoRoot ".env.integration")

if (-not $PSBoundParameters.ContainsKey("Profile") -and $env:ROVE_INTEGRATION_PROFILE) {
    $Profile = $env:ROVE_INTEGRATION_PROFILE
}
if (-not $PSBoundParameters.ContainsKey("ApiAddr") -and $env:ROVE_API_BIND_ADDR) {
    $ApiAddr = $env:ROVE_API_BIND_ADDR
}
if (-not $PSBoundParameters.ContainsKey("WebPort") -and $env:ROVE_WEB_PORT) {
    $WebPort = $env:ROVE_WEB_PORT
}
if (-not $PSBoundParameters.ContainsKey("IntegrationRoot") -and $env:ROVE_INTEGRATION_ROOT) {
    $IntegrationRoot = $env:ROVE_INTEGRATION_ROOT
}

Test-CommandAvailable "cargo"
Test-CommandAvailable "pnpm"

$IntegrationRoot = Resolve-RepoPath $IntegrationRoot
$WorkspaceDir = Resolve-RepoPath $(if ($env:ROVE_INTEGRATION_WORKSPACE) { $env:ROVE_INTEGRATION_WORKSPACE } else { Join-Path $IntegrationRoot "workspace" })
$StateDir = Resolve-RepoPath $(if ($env:ROVE_STATE_DIR) { $env:ROVE_STATE_DIR } else { Join-Path $WorkspaceDir ".rove-integration-state" })
$ArtifactsDir = Resolve-RepoPath $(if ($env:ROVE_INTEGRATION_ARTIFACTS) { $env:ROVE_INTEGRATION_ARTIFACTS } else { Join-Path $IntegrationRoot "artifacts" })
$ApiArtifacts = Join-Path $ArtifactsDir "api"
$ApiStdoutLog = Join-Path $ArtifactsDir "rove-api.out.log"
$ApiStderrLog = Join-Path $ArtifactsDir "rove-api.err.log"
$WebStdoutLog = Join-Path $ArtifactsDir "web.out.log"
$WebStderrLog = Join-Path $ArtifactsDir "web.err.log"
$ApiBase = "http://$ApiAddr"
$WebBase = "http://127.0.0.1:$WebPort"
$script:RunIds = @()
$apiProcess = $null
$webProcess = $null

try {
    if (-not $KeepState) {
        foreach ($path in @($WorkspaceDir)) {
            if (Test-Path -LiteralPath $path) {
                Remove-Item -LiteralPath $path -Recurse -Force
            }
        }
    }
    New-Item -ItemType Directory -Force -Path $WorkspaceDir, $StateDir, $ArtifactsDir, $ApiArtifacts | Out-Null

    $env:ROVE_PROVIDER = "fake"
    $env:ROVE_MODEL = "fake"
    $env:ROVE_STATE_DIR = ".rove-integration-state"
    $env:ROVE_STATE_SQLITE = ".rove-integration-state/state.sqlite"
    $env:ROVE_MEMORY_SESSION_DIR = ".rove-integration-state/memory/sessions"
    $env:ROVE_MEMORY_DURABLE_DIR = ".rove-integration-state/memory"
    $env:ROVE_API_BIND_ADDR = $ApiAddr
    $env:ROVE_API_BASE = $ApiBase
    if (-not $SkipWebE2E) {
        Add-CorsOrigins @($WebBase, "http://localhost:$WebPort")
    }

    $apiBuildArgs = @("build", "-p", "rove-api", "--bin", "rove-api")
    Push-Location $RepoRoot
    try {
        & cargo @apiBuildArgs
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build -p rove-api --bin rove-api failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }

    $apiBinaryName = if ($env:OS -eq "Windows_NT") { "rove-api.exe" } else { "rove-api" }
    $apiBinary = Join-Path (Join-Path (Join-Path $RepoRoot "target") "debug") $apiBinaryName
    $apiArgs = @("--addr", $ApiAddr, "-C", $WorkspaceDir)
    $apiProcess = Start-BackgroundCommand -Command $apiBinary -Arguments $apiArgs -WorkingDirectory $RepoRoot -StdoutLog $ApiStdoutLog -StderrLog $ApiStderrLog
    Wait-HttpOk -Uri "$ApiBase/runs?limit=1" -TimeoutSeconds 60 -Name "rove-api"

    if (-not $SkipApiSmoke) {
        Run-ApiSmoke
    }

    if (-not $SkipWebE2E) {
        $env:ROVE_REAL_API_E2E = "1"
        $env:ROVE_REAL_API_WORKBENCH_SMOKE = "1"
        $env:ROVE_WEB_PORT = $WebPort
        $env:PLAYWRIGHT_BASE_URL = $WebBase
        $env:PLAYWRIGHT_HTML_REPORT = Join-Path $ArtifactsDir "playwright-report"
        $env:PLAYWRIGHT_TEST_OUTPUT_DIR = Join-Path $ArtifactsDir "playwright-results"

        $webEnv = @{
            ROVE_API_BASE = $ApiBase
            ROVE_API_TOKEN = $env:ROVE_API_TOKEN
        }
        foreach ($entry in $webEnv.GetEnumerator()) {
            if ($null -ne $entry.Value) {
                [Environment]::SetEnvironmentVariable($entry.Key, [string]$entry.Value, "Process")
            }
        }

        $webProcess = Start-BackgroundCommand -Command "pnpm" -Arguments @("exec", "next", "dev", "--port", $WebPort) -WorkingDirectory (Join-Path $RepoRoot "apps/web") -StdoutLog $WebStdoutLog -StderrLog $WebStderrLog
        Wait-HttpOk -Uri $WebBase -TimeoutSeconds 120 -Name "web"

        Push-Location (Join-Path $RepoRoot "apps/web")
        try {
            $playwrightOutput = Join-Path $ArtifactsDir "playwright-results"
            pnpm exec playwright test tests/e2e/real-api.spec.ts --project=chromium --output $playwrightOutput
            if ($LASTEXITCODE -ne 0) {
                throw "real API Playwright suite failed with exit code $LASTEXITCODE"
            }
        } finally {
            Pop-Location
        }
    }

    Write-Host "local-full integration smoke completed"
    Write-Host "Artifacts: $ArtifactsDir"
    if ($script:RunIds.Count -gt 0) {
        Write-Host ("Run ids: " + ($script:RunIds -join ", "))
    }
} finally {
    if ($webProcess) {
        Stop-ProcessTree $webProcess
    }
    if ($apiProcess) {
        Stop-ProcessTree $apiProcess
    }
}
