param(
    [string]$ApiAddr = $(if ($env:ROVE_API_BIND_ADDR) { $env:ROVE_API_BIND_ADDR } else { "127.0.0.1:8787" }),
    [string]$WebPort = $(if ($env:ROVE_WEB_PORT) { $env:ROVE_WEB_PORT } else { "3000" }),
    [string]$Workspace = $(if ($env:ROVE_DEV_WORKSPACE) { $env:ROVE_DEV_WORKSPACE } else { (Split-Path -Parent $PSScriptRoot) }),
    [int]$RunSeconds = 0,
    [switch]$Provider,
    [switch]$InstallWebDeps
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

function Test-PortFree([int]$Port, [string]$Name) {
    $existing = Get-NetTCPConnection -LocalPort $Port -ErrorAction SilentlyContinue | Where-Object { $_.State -eq "Listen" }
    if ($existing) {
        throw "$Name port $Port is already in use. Stop the existing process or pass a different port."
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

function Get-PortFromAddress([string]$Address) {
    $portText = $Address.Split(":")[-1]
    return [int]$portText
}

$RepoRoot = Split-Path -Parent $PSScriptRoot
Import-DotEnv (Join-Path $RepoRoot ".env.integration")

Test-CommandAvailable "cargo"
Test-CommandAvailable "pnpm"

$apiPort = Get-PortFromAddress $ApiAddr
$webPortNumber = [int]$WebPort
Test-PortFree $apiPort "API"
Test-PortFree $webPortNumber "Web"

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
    Push-Location (Join-Path $RepoRoot "apps/web")
    try {
        pnpm install --frozen-lockfile
    } finally {
        Pop-Location
    }
}

$apiProcess = $null
$webProcess = $null

try {
    $apiProcess = Start-BackgroundCommand -Command "cargo" -Arguments @("run", "--bin", "rove-api", "--", "--addr", $ApiAddr, "-C", $Workspace) -WorkingDirectory $RepoRoot
    Wait-HttpOk -Uri "http://$ApiAddr/runs?limit=1" -TimeoutSeconds 60 -Name "rove-api"

    $webProcess = Start-BackgroundCommand -Command "pnpm" -Arguments @("exec", "next", "dev", "--port", $WebPort) -WorkingDirectory (Join-Path $RepoRoot "apps/web")
    Wait-HttpOk -Uri "http://localhost:$WebPort" -TimeoutSeconds 120 -Name "web"

    Write-Host "rove dev environment is running"
    Write-Host "Web:       http://localhost:$WebPort"
    Write-Host "API:       http://$ApiAddr"
    Write-Host "Workspace: $Workspace"
    Write-Host "State:     $env:ROVE_STATE_DIR"
    Write-Host "Provider:  $env:ROVE_PROVIDER"
    Write-Host "Model:     $env:ROVE_MODEL"
    Write-Host "Press Ctrl+C to stop API and Web."

    if ($RunSeconds -gt 0) {
        Start-Sleep -Seconds $RunSeconds
        return
    }

    while ($true) {
        Start-Sleep -Seconds 1
        if ($apiProcess.HasExited) {
            throw "rove-api exited with code $($apiProcess.ExitCode)"
        }
        if ($webProcess.HasExited) {
            throw "web exited with code $($webProcess.ExitCode)"
        }
    }
} finally {
    if ($webProcess) {
        Stop-ProcessTree $webProcess
    }
    if ($apiProcess) {
        Stop-ProcessTree $apiProcess
    }
}
