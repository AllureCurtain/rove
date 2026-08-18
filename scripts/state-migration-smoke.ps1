# Requires -Version 5.1
<#
.SYNOPSIS
    Windows-rerunnable smoke for the user state directory migration:
    legacy `.rove/` discovery, dry-run side-effect freedom, apply,
    idempotency, conflict handling, prune, and post-migration path
    inspection. Uses only the locally built `rove` CLI and a temp sandbox.

.DESCRIPTION
    The script never touches the real user data root; ROVE_DATA_ROOT is
    pointed at a temp directory for every invocation. Exit code 0 means
    every scenario passed; any failure aborts with a non-zero exit.
#>
[CmdletBinding()]
param(
    [string]$CargoRoot = "D:\Program\Rust\.cargo\bin"
)

$ErrorActionPreference = 'Stop'
if ($CargoRoot -and (Test-Path $CargoRoot)) {
    $env:PATH = "$CargoRoot;$env:PATH"
}

$RepoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepoRoot

# Build the CLI first so failures are loud and early.
$PreviousErrorActionPreference = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
try {
    # Windows PowerShell 5.1 wraps native stderr as ErrorRecord objects.
    # Temporarily avoid promoting ordinary Cargo progress to a terminating
    # error; the native exit code remains the authoritative result.
    & cargo build -p rove-cli 2>$null
    $BuildExitCode = $LASTEXITCODE
} finally {
    $ErrorActionPreference = $PreviousErrorActionPreference
}
if ($BuildExitCode -ne 0) { throw "cargo build -p rove-cli failed with exit $BuildExitCode" }
$Rove = Join-Path $RepoRoot "target\debug\rove.exe"
if (-not (Test-Path $Rove)) { throw "built rove.exe not found at $Rove" }

$Sandbox = Join-Path ([System.IO.Path]::GetTempPath()) ("rove-migration-smoke-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $Sandbox | Out-Null
$DataRoot = Join-Path $Sandbox 'data-root'
$Workspace = Join-Path $Sandbox 'ws'
New-Item -ItemType Directory -Path $Workspace | Out-Null

$script:Failures = 0
function Assert-True($Condition, $Message) {
    if (-not $Condition) {
        Write-Host "FAIL: $Message" -ForegroundColor Red
        $script:Failures++
    } else {
        Write-Host "ok:   $Message" -ForegroundColor Green
    }
}

function Invoke-Rove($Arguments) {
    $env:ROVE_DATA_ROOT = $DataRoot
    $PreviousErrorActionPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        # Keep every CLI operation bound to the disposable workspace. The
        # build still runs from the repository root, but migration discovery
        # must never inspect the repository's own .rove directory.
        & $Rove '-C' $Workspace @Arguments 2>$null
        $ExitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $PreviousErrorActionPreference
        Remove-Item Env:\ROVE_DATA_ROOT -ErrorAction SilentlyContinue
    }
    return @{ ExitCode = $ExitCode }
}

try {
    # --- Scenario 1: fresh workspace, paths resolve into the data root.
    $result = Invoke-Rove @('state', 'paths')
    Assert-True ($result.ExitCode -eq 0) "state paths exits 0 on a fresh workspace"
    Assert-True (-not (Test-Path (Join-Path $Workspace '.rove'))) "fresh workspace does not create .rove"
    Assert-True (-not (Test-Path $DataRoot)) "state paths does not materialize the data root"

    # --- Scenario 2: legacy layout discovery + dry-run purity.
    $Legacy = Join-Path $Workspace '.rove'
    New-Item -ItemType Directory -Path (Join-Path $Legacy 'memory\sessions') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $Legacy 'runs\01J') -Force | Out-Null
    Set-Content (Join-Path $Legacy 'config.toml') '[state]'
    Set-Content (Join-Path $Legacy 'mcp_servers.json') '{"servers":[]}'
    Set-Content (Join-Path $Legacy 'memory\MEMORY.md') '# memory'
    Set-Content (Join-Path $Legacy 'runs\01J\trace.jsonl') '{}'

    $result = Invoke-Rove @('state', 'migrate')
    Assert-True ($result.ExitCode -eq 0) "dry-run exits 0"
    Assert-True (-not (Test-Path $DataRoot)) "dry-run writes nothing to the data root"

    # --- Scenario 3: apply + idempotency.
    $result = Invoke-Rove @('state', 'migrate', '--apply')
    Assert-True ($result.ExitCode -eq 0) "apply exits 0"
    Assert-True (Test-Path (Join-Path $DataRoot 'workspaces')) "apply creates workspaces/ under the data root"
    Assert-True (Test-Path (Join-Path $Legacy 'config.toml')) "project config stays in .rove"
    Assert-True (Test-Path (Join-Path $Legacy 'mcp_servers.json')) "apply keeps the legacy source by default"

    $second = Invoke-Rove @('state', 'migrate', '--apply')
    Assert-True ($second.ExitCode -eq 0) "second apply exits 0 (idempotent)"

    # --- Scenario 4: conflict is visible and keeps the target.
    $WorkspaceDir = Get-ChildItem (Join-Path $DataRoot 'workspaces') -Directory | Select-Object -First 1
    Set-Content (Join-Path $WorkspaceDir.FullName 'memory\MEMORY.md') 'conflicting'
    $conflict = Invoke-Rove @('state', 'migrate', '--apply')
    Assert-True ($conflict.ExitCode -ne 0) "unresolved conflict exits non-zero"
    $kept = Get-Content (Join-Path $WorkspaceDir.FullName 'memory\MEMORY.md')
    Assert-True ($kept -eq 'conflicting') "keep-target policy does not overwrite"

    $resolved = Invoke-Rove @('state', 'migrate', '--apply', '--on-conflict', 'backup-target')
    Assert-True ($resolved.ExitCode -eq 0) "backup-target resolves the conflict"
    $replaced = Get-Content (Join-Path $WorkspaceDir.FullName 'memory\MEMORY.md')
    Assert-True ($replaced -eq '# memory') "backup-target copies the source"
    Assert-True ((Get-ChildItem (Join-Path $WorkspaceDir.FullName '.migration\conflicts') -ErrorAction SilentlyContinue | Measure-Object).Count -ge 1) "conflict backup is kept"

    # --- Scenario 5: prune removes migrated files, never project config.
    $prune = Invoke-Rove @('state', 'migrate', '--apply', '--prune-legacy')
    Assert-True ($prune.ExitCode -eq 0) "prune exits 0 after a clean apply"
    Assert-True (Test-Path (Join-Path $Legacy 'config.toml')) "prune never deletes project config"
    Assert-True (-not (Test-Path (Join-Path $Legacy 'mcp_servers.json'))) "prune removes migrated files"

    # --- Scenario 6: path inspection keeps working after migration.
    $paths = Invoke-Rove @('state', 'paths')
    Assert-True ($paths.ExitCode -eq 0) "state paths still exits 0 after migration"
}
finally {
    Remove-Item -Recurse -Force $Sandbox -ErrorAction SilentlyContinue
}

if ($Failures -gt 0) {
    Write-Host "state-migration-smoke: $Failures failure(s)" -ForegroundColor Red
    exit 1
}
Write-Host "state-migration-smoke: all scenarios passed" -ForegroundColor Green
exit 0
