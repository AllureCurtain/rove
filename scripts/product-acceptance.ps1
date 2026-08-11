param(
    [string]$ReportPath = $(if ($env:ROVE_ACCEPTANCE_REPORT) { $env:ROVE_ACCEPTANCE_REPORT } else { "PRODUCT_ACCEPTANCE_REPORT.json" }),
    [switch]$SkipWeb,
    [switch]$SkipBrowser,
    [switch]$IncludeGated
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
$WebRoot = Join-Path $RepoRoot "apps/web"

# Every entry is executed. Nothing here may be reported as passing without a
# real exit code, and anything skipped is recorded as not_run with a reason.
$Checks = @(
    @{ id = "fmt"; group = "gate"; description = "Rust formatting"; command = "cargo"; arguments = @("fmt", "--all", "--check"); cwd = $RepoRoot; required = $true }
    @{ id = "clippy"; group = "gate"; description = "Rust lints as errors"; command = "cargo"; arguments = @("clippy", "--workspace", "--all-targets", "--", "-D", "warnings"); cwd = $RepoRoot; required = $true }
    @{ id = "test-api"; group = "G1-G7"; description = "API contract suite"; command = "cargo"; arguments = @("test", "-p", "rove-integration-tests", "--test", "api", "--", "--test-threads=1"); cwd = $RepoRoot; required = $true }
    @{ id = "test-mcp"; group = "G7"; description = "MCP transport and hardening"; command = "cargo"; arguments = @("test", "-p", "rove-integration-tests", "--test", "mcp"); cwd = $RepoRoot; required = $true }
    @{ id = "test-e2e"; group = "G1-G4"; description = "Engine and planner loop"; command = "cargo"; arguments = @("test", "-p", "rove-integration-tests", "--test", "e2e"); cwd = $RepoRoot; required = $true }
    @{ id = "test-tool-safety"; group = "G5"; description = "Tool safety boundaries"; command = "cargo"; arguments = @("test", "-p", "rove-integration-tests", "--test", "tool_safety"); cwd = $RepoRoot; required = $true }
    @{ id = "test-product-store"; group = "G1-G6"; description = "Product store persistence"; command = "cargo"; arguments = @("test", "-p", "rove-api", "--lib", "product::", "--", "--test-threads=1"); cwd = $RepoRoot; required = $true }
    @{ id = "web-typecheck"; group = "G3-G7"; description = "Web TypeScript"; command = "pnpm"; arguments = @("typecheck"); cwd = $WebRoot; required = $true; web = $true }
    @{ id = "web-test"; group = "G3-G7"; description = "Web unit and component tests"; command = "pnpm"; arguments = @("test"); cwd = $WebRoot; required = $true; web = $true }
    @{ id = "web-build"; group = "G3-G7"; description = "Web production build"; command = "pnpm"; arguments = @("build"); cwd = $WebRoot; required = $true; web = $true }
    @{ id = "web-e2e"; group = "G1-G7"; description = "Browser-boundary Playwright suites"; command = "pnpm"; arguments = @("test:e2e"); cwd = $WebRoot; required = $true; web = $true; browser = $true }
    @{ id = "mcp-filesystem-smoke"; group = "G7"; description = "Real MCP filesystem server smoke"; command = "cargo"; arguments = @("test", "-p", "rove-integration-tests", "--test", "mcp", "mcp_official_filesystem_server_smoke_when_enabled", "--", "--exact", "--nocapture"); cwd = $RepoRoot; required = $false; gated = $true; gateEnv = "ROVE_MCP_FILESYSTEM_SMOKE" }
)

function Get-GitValue([string[]]$Arguments, [string]$Fallback) {
    try {
        $value = (& git @Arguments 2>$null | Out-String).Trim()
        if ($LASTEXITCODE -ne 0 -or -not $value) {
            return $Fallback
        }
        return $value
    } catch {
        return $Fallback
    }
}

function Get-ToolVersion([string]$Command, [string[]]$Arguments) {
    if (-not (Get-Command $Command -ErrorAction SilentlyContinue)) {
        return $null
    }
    try {
        $value = (& $Command @Arguments 2>$null | Out-String).Trim()
        if (-not $value) {
            return $null
        }
        return ($value -split "`n")[0].Trim()
    } catch {
        return $null
    }
}

# [System.IO.Path]::GetRelativePath does not exist on Windows PowerShell 5.1.
function Get-RelativePathCompat([string]$Base, [string]$Path) {
    $baseFull = [System.IO.Path]::GetFullPath($Base).TrimEnd("\", "/")
    $pathFull = [System.IO.Path]::GetFullPath($Path).TrimEnd("\", "/")
    if ($pathFull -eq $baseFull) {
        return "."
    }
    if ($pathFull.StartsWith($baseFull + [System.IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        return $pathFull.Substring($baseFull.Length + 1).Replace("\", "/")
    }
    return $pathFull.Replace("\", "/")
}

function Get-OutputTail([string]$Path, [int]$Lines = 40) {
    if (-not (Test-Path -LiteralPath $Path)) {
        return @()
    }
    $content = Get-Content -LiteralPath $Path -ErrorAction SilentlyContinue
    if (-not $content) {
        return @()
    }
    # Get-Content strings carry provider metadata in Windows PowerShell. Force
    # plain strings so ConvertTo-Json cannot serialize the whole provider graph.
    return @($content | Select-Object -Last $Lines | ForEach-Object { $_.ToString() })
}

$LogDir = Join-Path $RepoRoot ".rove/acceptance-logs"
if (Test-Path -LiteralPath $LogDir) {
    Remove-Item -LiteralPath $LogDir -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $LogDir | Out-Null

$results = [System.Collections.Generic.List[object]]::new()
$startedAt = Get-Date

foreach ($check in $Checks) {
    $id = $check.id
    $isWeb = $check.ContainsKey("web") -and $check.web
    $isBrowser = $check.ContainsKey("browser") -and $check.browser
    $isGated = $check.ContainsKey("gated") -and $check.gated
    $required = $check.required

    $skipReason = $null
    if ($isWeb -and $SkipWeb) {
        $skipReason = "-SkipWeb was requested"
    } elseif ($isBrowser -and $SkipBrowser) {
        $skipReason = "-SkipBrowser was requested"
    } elseif ($isGated -and -not $IncludeGated) {
        $skipReason = "gated check; pass -IncludeGated to run it"
    } elseif ($isGated -and $IncludeGated -and $check.gateEnv -and -not [Environment]::GetEnvironmentVariable($check.gateEnv)) {
        $skipReason = "gate variable $($check.gateEnv) is not set"
    } elseif (-not (Get-Command $check.command -ErrorAction SilentlyContinue)) {
        $skipReason = "required command '$($check.command)' was not found on PATH"
    }

    $commandLine = ($check.command + " " + ($check.arguments -join " ")).Trim()

    if ($skipReason) {
        Write-Host "not_run  $id : $skipReason"
        $results.Add([ordered]@{
            id = $id
            group = $check.group
            description = $check.description
            command = $commandLine
            working_directory = Get-RelativePathCompat $RepoRoot $check.cwd
            required = $required
            status = "not_run"
            reason = $skipReason
            exit_code = $null
            duration_seconds = $null
            output_tail = @()
        })
        continue
    }

    $stdoutLog = Join-Path $LogDir "$id.out.log"
    $stderrLog = Join-Path $LogDir "$id.err.log"
    Write-Host "running  $id : $commandLine"

    $checkStart = Get-Date
    # Wait only for the command process. Start-Process -Wait follows the entire
    # descendant tree on Windows, so a test fixture that intentionally detaches
    # a bounded helper can delay or hang the acceptance runner after Cargo has
    # already returned its real exit code.
    $resolvedCommand = Get-Command $check.command -ErrorAction Stop
    $processFilePath = $resolvedCommand.Path
    $processArguments = @($check.arguments)
    if ($resolvedCommand.CommandType -eq "ExternalScript") {
        $processFilePath = (Get-Command "powershell" -ErrorAction Stop).Path
        $processArguments = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $resolvedCommand.Path) + $processArguments
    }
    $process = Start-Process -FilePath $processFilePath -ArgumentList $processArguments `
        -WorkingDirectory $check.cwd -RedirectStandardOutput $stdoutLog -RedirectStandardError $stderrLog `
        -PassThru -NoNewWindow
    $process.WaitForExit()
    $process.Refresh()
    $exitCode = $process.ExitCode
    $duration = [Math]::Round(((Get-Date) - $checkStart).TotalSeconds, 2)

    # Status is derived only from a real exit code. There is no default pass, and
    # an undeterminable exit code is an error rather than a silent pass or fail.
    if ($null -eq $exitCode) {
        $status = "error"
        $reason = "the process exit code could not be determined"
    } elseif ($exitCode -eq 0) {
        $status = "pass"
        $reason = $null
    } else {
        $status = "fail"
        $reason = $null
    }
    $tail = if ($status -eq "pass") { Get-OutputTail $stdoutLog 15 } else { @(Get-OutputTail $stdoutLog 40) + @(Get-OutputTail $stderrLog 40) }

    Write-Host "$status  $id (exit $exitCode, ${duration}s)"
    $results.Add([ordered]@{
        id = $id
        group = $check.group
        description = $check.description
        command = $commandLine
        working_directory = Get-RelativePathCompat $RepoRoot $check.cwd
        required = $required
        status = $status
        reason = $reason
        exit_code = $exitCode
        duration_seconds = $duration
        output_tail = @($tail)
    })
}

$failed = @($results | Where-Object { $_.status -eq "fail" -or $_.status -eq "error" })
$requiredNotRun = @($results | Where-Object { $_.status -eq "not_run" -and $_.required })
$passed = @($results | Where-Object { $_.status -eq "pass" })

# PASS requires zero failures and zero unrun required checks.
$verdict = if ($failed.Count -eq 0 -and $requiredNotRun.Count -eq 0) { "PASS" } else { "FAIL" }

$dirtyOutput = Get-GitValue @("status", "--porcelain") ""
$report = [ordered]@{
    schema_version = 1
    report = "product-acceptance"
    generated_at = $startedAt.ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    duration_seconds = [Math]::Round(((Get-Date) - $startedAt).TotalSeconds, 2)
    verdict = $verdict
    source = [ordered]@{
        branch = Get-GitValue @("rev-parse", "--abbrev-ref", "HEAD") "unknown"
        commit = Get-GitValue @("rev-parse", "HEAD") "unknown"
        dirty = [bool]$dirtyOutput
        dirty_file_count = @($dirtyOutput -split "`n" | Where-Object { $_.Trim() }).Count
    }
    environment = [ordered]@{
        os = [System.Runtime.InteropServices.RuntimeInformation]::OSDescription.Trim()
        cargo = Get-ToolVersion "cargo" @("--version")
        rustc = Get-ToolVersion "rustc" @("--version")
        node = Get-ToolVersion "node" @("--version")
        pnpm = Get-ToolVersion "pnpm" @("--version")
    }
    totals = [ordered]@{
        total = $results.Count
        passed = $passed.Count
        failed = $failed.Count
        not_run = @($results | Where-Object { $_.status -eq "not_run" }).Count
        required_not_run = $requiredNotRun.Count
    }
    checks = @($results)
}

$ReportPath = if ([System.IO.Path]::IsPathRooted($ReportPath)) { $ReportPath } else { Join-Path $RepoRoot $ReportPath }
# Write UTF-8 without a BOM: Windows PowerShell's -Encoding utf8 emits a BOM,
# which strict JSON parsers reject.
$json = $report | ConvertTo-Json -Depth 8
[System.IO.File]::WriteAllText($ReportPath, $json + "`n", (New-Object System.Text.UTF8Encoding($false)))

Write-Host ""
Write-Host "verdict: $verdict ($($passed.Count) passed, $($failed.Count) failed, $($results.Count - $passed.Count - $failed.Count) not run)"
Write-Host "report:  $ReportPath"
Write-Host "logs:    $LogDir"

if ($verdict -ne "PASS") {
    foreach ($entry in $failed) {
        Write-Host "  FAILED  $($entry.id) (exit $($entry.exit_code))"
    }
    foreach ($entry in $requiredNotRun) {
        Write-Host "  NOT RUN $($entry.id): $($entry.reason)"
    }
    exit 1
}
exit 0
