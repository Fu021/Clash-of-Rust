#Requires -Version 5.1
<#
.SYNOPSIS
  Per-user WSL egress broker. Read-only unless -Execute is supplied.
.DESCRIPTION
  Emits one schema-versioned JSON response. Never elevates, invokes SSH, shuts
  down WSL, or stops the shared Mihomo core. Import the module for mock tests.
  A disconnected transport leaves the persistent Windows guard in place.
#>
[CmdletBinding()]
param(
    [ValidateSet('Doctor','Status','Install','Start','Stop','Test','Rollback','Uninstall')]
    [string]$Action = 'Doctor',
    [ValidateSet('Tailscale','WireGuard')][string]$Backend = 'Tailscale',
    [string]$Distro = 'COR-Egress',
    [string]$ProfilePath,
    [string]$EndpointIPv4,
    [ValidateRange(1,65535)][int]$EndpointPort = 51820,
    [switch]$Execute,
    [switch]$RestoreOriginalNetwork
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'WslSecureEgress.psm1') -Force -DisableNameChecking
try {
    $response = Invoke-WslEgressOperation @PSBoundParameters
    $response | ConvertTo-Json -Depth 12 -Compress
    if ($response.outcome -eq 'failed') { exit 1 }
    if ($response.outcome -eq 'blocked') { exit 2 }
    exit 0
} catch {
    # Exceptions may contain private configuration. Never print their text.
    New-WslEgressResponse -Action $Action -Backend $Backend -Outcome failed `
        -Phase failed -Message 'Operation failed; inspect the protected local journal and restore only owned objects.' |
        ConvertTo-Json -Depth 12 -Compress
    exit 1
}
