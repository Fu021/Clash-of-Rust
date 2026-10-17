#Requires -Version 5.1
param([switch]$Execute)
# Installer safety check. Never remove an active guard or its offline recovery.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
try {
    Import-Module (Join-Path $PSScriptRoot 'WslSecureEgress.psm1') -Force -DisableNameChecking
    $root = Join-Path $env:ProgramData 'ClashOfRust\WslEgress'
    $path = Join-Path $root 'state.json'
    $service = Get-CimInstance Win32_Service -Filter "Name='ClashOfRust.WslEgress'"
    if (Test-Path -LiteralPath $path) {
        $state = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
        if ($state.owner -ne 'ClashOfRust.WslEgress' -or $state.phase -ne 'restored') { exit 2 }
    } elseif ($service) { exit 2 }
    if ($service -and $Execute) {
        # The guardian service must run from a protected package directory under
        # Program Files: either the helper package or an application package
        # installed there. The helper package is the documented layout, so a
        # check against this script's own copy would block every real uninstall.
        if ($service.StartName -notin @('LocalSystem', 'NT AUTHORITY\SYSTEM')) { exit 2 }
        if (-not (Test-WslEgressGuardianServicePath ([string]$service.PathName))) { exit 2 }
        Stop-Service -Name 'ClashOfRust.WslEgress' -ErrorAction Stop
        & (Join-Path $env:SystemRoot 'System32\sc.exe') delete 'ClashOfRust.WslEgress' | Out-Null
        if ($LASTEXITCODE -ne 0) { exit 1 }
    }
    exit 0
} catch { exit 1 }
