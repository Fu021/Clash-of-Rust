#Requires -Version 5.1
# Fixed SYSTEM action. No WSL enumeration, per-user process, or network mutation.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'WslSecureEgress.psm1') -Force -DisableNameChecking
try { Invoke-WslEgressGuardianAudit; exit 0 } catch { exit 1 }
