param(
    [string]$ApiAddr = $(if ($env:ROVE_API_BIND_ADDR) { $env:ROVE_API_BIND_ADDR } else { "127.0.0.1:8787" }),
    [string]$Workspace = $(if ($env:ROVE_DEV_WORKSPACE) { $env:ROVE_DEV_WORKSPACE } else { (Split-Path -Parent $PSScriptRoot) }),
    [switch]$Provider,
    [switch]$SkipWebBuild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

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

$RepoRoot = Split-Path -Parent $PSScriptRoot
$WebDist = Join-Path $RepoRoot "apps/web/web-dist"
Import-DotEnv (Join-Path $RepoRoot ".env")

Test-CommandAvailable "cargo"

# Product mode: one process serves the built console and the API on the same
# origin, unlike dev.ps1 which runs next dev as a second process. The bundle
# is rebuilt here so the script stays a single command; -SkipWebBuild reuses
# an existing apps/web/web-dist for fast API-only iterations.
if (-not $SkipWebBuild -or -not (Test-Path -LiteralPath (Join-Path $WebDist "index.html"))) {
    if (-not (Get-Command "pnpm" -ErrorAction SilentlyContinue)) {
        throw "Required command 'pnpm' was not found on PATH (needed to build the web bundle; or pass -SkipWebBuild with an existing apps/web/web-dist)."
    }
    Write-Host "building web bundle -> apps/web/web-dist"
    Push-Location (Join-Path $RepoRoot "apps/web")
    try {
        pnpm build:web
        if ($LASTEXITCODE -ne 0) {
            throw "pnpm build:web failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
}

$Workspace = [System.IO.Path]::GetFullPath($Workspace)
New-Item -ItemType Directory -Force -Path $Workspace | Out-Null

if (-not $Provider) {
    $env:ROVE_PROVIDER = "fake"
    $env:ROVE_MODEL = "fake"
}

Write-Host "rove console: http://$ApiAddr/"
Write-Host "workspace:    $Workspace"
cargo run -p rove-api --bin rove-api -- --addr $ApiAddr -C $Workspace --web-dist $WebDist
exit $LASTEXITCODE
