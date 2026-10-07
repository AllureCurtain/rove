param(
    [string]$ApiAddr = $(if ($env:ROVE_SERVE_ACCEPTANCE_ADDR) { $env:ROVE_SERVE_ACCEPTANCE_ADDR } else { "127.0.0.1:18787" }),
    [switch]$SkipWebBuild,
    [switch]$SkipCargoBuild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Static-hosted acceptance: build the console bundle, serve it from
# `rove-api --web-dist` on one origin, and run the real-API Playwright suite
# against that process — the same form `scripts/serve.ps1` hands to a user.
# Nothing here touches the operator's data root or trust store; both are
# pinned to a scratch directory for the spawned server.

$RepoRoot = Split-Path -Parent $PSScriptRoot
$WebRoot = Join-Path $RepoRoot "apps/web"
$WebDist = Join-Path $WebRoot "web-dist"

function Test-CommandAvailable([string]$Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required command '$Name' was not found on PATH."
    }
}

Test-CommandAvailable "pnpm"
Test-CommandAvailable "cargo"

if (-not $SkipWebBuild -or -not (Test-Path -LiteralPath (Join-Path $WebDist "index.html"))) {
    Write-Host "building web bundle -> apps/web/web-dist"
    Push-Location $WebRoot
    try {
        pnpm build:web
        if ($LASTEXITCODE -ne 0) {
            throw "pnpm build:web failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
}

if (-not $SkipCargoBuild) {
    Write-Host "building rove-api"
    Push-Location $RepoRoot
    try {
        cargo build -p rove-api --bin rove-api
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build -p rove-api failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
}

# `.cargo/config.toml` pins target-dir relative to the repository root, so
# worktree checkouts share one build cache. Ask Cargo where that is instead of
# assuming `$RepoRoot/target`.
$metadata = (cargo metadata --no-deps --format-version 1 --manifest-path (Join-Path $RepoRoot "Cargo.toml") | ConvertFrom-Json)
$TargetDir = Join-Path $metadata.target_directory "debug"
$ApiBin = Join-Path $TargetDir "rove-api.exe"
if (-not (Test-Path -LiteralPath $ApiBin)) {
    $ApiBin = Join-Path $TargetDir "rove-api"
}
if (-not (Test-Path -LiteralPath $ApiBin)) {
    throw "rove-api binary not found under $TargetDir (build it or drop -SkipCargoBuild)."
}

$Scratch = Join-Path ([System.IO.Path]::GetTempPath()) ("rove-serve-acceptance-" + [System.Guid]::NewGuid().ToString("N"))
$Workspace = Join-Path $Scratch "workspace"
$DataRoot = Join-Path $Scratch "data"
$LogOut = Join-Path $Scratch "rove-api.out.log"
$LogErr = Join-Path $Scratch "rove-api.err.log"
New-Item -ItemType Directory -Force -Path $Workspace, $DataRoot | Out-Null

# Start-Process has no env map (PowerShell 5.1); it inherits the caller's
# environment, so scope the server vars through the current process and
# restore them in the finally block.
$scopedEnv = @{
    ROVE_PROVIDER = "fake"
    ROVE_MODEL = "fake"
    ROVE_DATA_ROOT = $DataRoot
    ROVE_PROJECT_TRUST_STORE = (Join-Path $DataRoot "project-trust.sqlite")
}
$savedEnv = @{}

$server = $null
try {
    foreach ($key in $scopedEnv.Keys) {
        $savedEnv[$key] = [Environment]::GetEnvironmentVariable($key, "Process")
        [Environment]::SetEnvironmentVariable($key, $scopedEnv[$key], "Process")
    }
    $argumentList = @("--addr", $ApiAddr, "-C", $Workspace, "--web-dist", $WebDist)
    $server = Start-Process -FilePath $ApiBin -ArgumentList $argumentList `
        -WorkingDirectory $RepoRoot `
        -RedirectStandardOutput $LogOut -RedirectStandardError $LogErr `
        -PassThru -NoNewWindow

    $healthUrl = "http://$ApiAddr/health"
    $deadline = (Get-Date).AddSeconds(60)
    $ready = $false
    while ((Get-Date) -lt $deadline) {
        if ($server.HasExited) {
            $stderr = Get-Content -LiteralPath $LogErr -Raw -ErrorAction SilentlyContinue
            throw "rove-api exited before becoming ready:`n$stderr"
        }
        try {
            $health = Invoke-RestMethod -Uri $healthUrl -TimeoutSec 2
            if ($health.status -eq "ok") {
                $ready = $true
                break
            }
        } catch {
            Start-Sleep -Milliseconds 500
        }
    }
    if (-not $ready) {
        throw "rove-api did not answer $healthUrl within 60s"
    }
    Write-Host "rove-api serving $WebDist at http://$ApiAddr/"

    $env:ROVE_REAL_API_E2E = "1"
    $env:PLAYWRIGHT_BASE_URL = "http://$ApiAddr"
    Push-Location $WebRoot
    try {
        pnpm exec playwright test tests/e2e/real-api.spec.ts
        if ($LASTEXITCODE -ne 0) {
            throw "real-api e2e failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
    Write-Host "serve-form acceptance passed: the hosted bundle drove the real API end to end"
} finally {
    if ($null -ne $server -and -not $server.HasExited) {
        Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
    }
    foreach ($key in $scopedEnv.Keys) {
        [Environment]::SetEnvironmentVariable($key, $savedEnv[$key], "Process")
    }
    Remove-Item -LiteralPath $Scratch -Recurse -Force -ErrorAction SilentlyContinue
}
