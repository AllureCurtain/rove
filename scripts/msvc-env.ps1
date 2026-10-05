# Load the MSVC x64 build environment into the current PowerShell session.
#
# Dot-source it, then run cargo in the same session:
#
#     . scripts/msvc-env.ps1
#     cargo test --workspace
#
# Needed when cargo cannot find the MSVC linker on its own. That happens when
# vswhere does not report a Visual Studio instance, or when Git's GNU
# `link.exe` is earlier on PATH. Only this session's environment changes.
# Override the vcvars location with ROVE_VCVARS64.

$vcvars = $env:ROVE_VCVARS64
if (-not $vcvars) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path -LiteralPath $vswhere) {
        $installation = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($installation) {
            $vcvars = Join-Path $installation "VC\Auxiliary\Build\vcvars64.bat"
        }
    }
}
if (-not $vcvars -or -not (Test-Path -LiteralPath $vcvars)) {
    throw "vcvars64.bat not found. Set ROVE_VCVARS64 to its full path, for example <VS install>\VC\Auxiliary\Build\vcvars64.bat."
}

cmd /c "`"$vcvars`" >nul 2>&1 && set" | ForEach-Object {
    if ($_ -match '^([^=]+)=(.*)$') {
        Set-Item -LiteralPath "env:$($Matches[1])" -Value $Matches[2]
    }
}

$linker = (Get-Command link.exe -ErrorAction SilentlyContinue).Source
if (-not $linker -or $linker -notlike "*\VC\Tools\MSVC\*") {
    throw "MSVC link.exe is still not first on PATH after loading $vcvars (found: $linker)."
}
Write-Host "MSVC environment loaded: $linker"
