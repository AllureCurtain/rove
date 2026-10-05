[CmdletBinding()]
param(
    # The Visual Studio instance's own install root on the machine being repaired,
    # for example "C:\Program Files\Microsoft Visual Studio\2022\Professional".
    [Parameter(Mandatory = $true)]
    [string]$InstallationPath
)

$ErrorActionPreference = "Stop"

# Root fix for the broken Visual Studio 2022 instance registration.
# Run elevated (one UAC prompt). Already executed on this machine 2026-10-01.
#
# Diagnosis (2026-10-01):
#   - VS Pro 2022 17.13.6 lives at its own install root (the value passed in as
#     -InstallationPath), but the machine-wide instance catalog was missing, so
#     the Setup Configuration API returned no instances and rustc could not
#     discover the MSVC linker.
#   - HKLM\SOFTWARE\Microsoft\VisualStudio\Setup\CachePath pointed at a
#     `VisualStudio\cache` folder that no longer existed. The
#     Setup provider resolves the machine catalog under that override, so the
#     catalog had to live under a deleted directory.
#   - The leftover per-user state.json (LOCALAPPDATA ...\_Instances\51c761b9)
#     had lost its identity fields; restored separately (user-writable).
#
# Effect after running:
#   - cargo/rustc link against the real MSVC toolchain in Git Bash, pwsh, and a
#     clean PATH without loading scripts/msvc-env.ps1 first (verified by
#     rebuilding rove-cli and inspecting the PDB, which records
#     <install root>\VC\Tools\MSVC\...\link.exe).
#   - vswhere still lists no instance: the Setup API enumerator needs more
#     registration data than this catalog alone. If a VS Installer Repair ever
#     succeeds, it will rebuild the full registration.
#
# This elevated script:
#   1. Removes the dead CachePath override so the provider falls back to the
#      default %ProgramData%\Microsoft\VisualStudio\Packages root.
#   2. (Re)writes the machine catalog entry there.
#   3. Runs vswhere to show the (still empty) enumeration result.

$k = "HKLM:\SOFTWARE\Microsoft\VisualStudio\Setup"
if (Get-ItemProperty $k -Name CachePath -ErrorAction SilentlyContinue) {
    Remove-ItemProperty -Path $k -Name CachePath
    Write-Host "removed dead CachePath override"
} else {
    Write-Host "CachePath override not present (already clean)"
}

$dir = "C:\ProgramData\Microsoft\VisualStudio\Packages\_Instances\51c761b9"
New-Item -ItemType Directory -Force $dir | Out-Null

$state = [ordered]@{
    installationName      = "VisualStudio/17.13.6+35931.197"
    installationPath      = $InstallationPath
    installationVersion   = "17.13.35931.197"
    productId             = "Microsoft.VisualStudio.Product.Professional"
    productPath           = (Join-Path $InstallationPath "Common7\IDE\devenv.exe")
    channelPath           = "C:\ProgramData\Microsoft\VisualStudio\Packages\_Channels\f01d07da\channelManifest.json"
    channelUri            = "https://aka.ms/vs/17/release/channel"
    channelId             = "VisualStudio.17.Release"
    enginePath            = "C:\Program Files (x86)\Microsoft Visual Studio\Installer"
    releaseVersion        = "17.13"
    isComplete            = $true
    isLaunchable          = $true
    isReinstallable       = $true
    isPrerelease          = $false
    isLocalCache          = $false
    isRepairable          = $true
    installationErrors    = @()
    installationWarnings  = @()
    downloadCache          = "C:\ProgramData\Microsoft\VisualStudio\Packages"
}
$state | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath "$dir\state.json" -Encoding UTF8
Write-Host "wrote $dir\state.json"

Write-Host "--- vswhere after fix ---"
& "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" -all -prerelease -products * -format json
