#Requires -Version 5.1
<#
.SYNOPSIS
  Reviewed live payload upgrade for one installed WSL Secure Egress generation.
.DESCRIPTION
  Preview by default. With -Execute, the transport must already be closed with
  -Action Stop; this script stages the current reviewed Linux payload, performs
  the installer's atomic reviewed upgrade, reruns manager repair-config, and
  audits the owned WSL configuration. It never enables the Windows endpoint and
  never touches WSL distributions other than the recorded manager distribution.
#>
[CmdletBinding()]
param(
    [switch]$Execute,
    [string]$Distro
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $Execute) {
    [ordered]@{
        schema_version = 1
        action = 'update-live-payload'
        outcome = 'not_run'
        message = 'Preview only. Run Stop -Execute first, then rerun this script as administrator with -Execute.'
        opens_endpoint = $false
    } | ConvertTo-Json -Compress
    exit 0
}
$module = Import-Module (Join-Path $PSScriptRoot 'WslSecureEgress.psm1') -Force -PassThru -DisableNameChecking
try {
    $result = & $module {
        param($RequestedDistro)
        if (-not (Test-WslEgressAdministrator)) {
            throw 'Administrator required for the reviewed payload upgrade.'
        }
        $sid = Assert-WslEgressUser
        $state = Read-WslEgressState
        if (-not $state) { throw 'Not installed.' }
        if ($state.owner_sid -ne $sid) { throw 'State belongs to a different Windows user.' }
        if ($state.phase -ne 'installed_closed') {
            throw 'Run the Stop action first; the reviewed payload upgrade requires the closed phase.'
        }
        if (-not (Get-WslEgressClosureAudit $state).confirmed) {
            throw 'Endpoint closure is not confirmed; payload upgrade refused.'
        }
        $distro = if ($RequestedDistro) { [string]$RequestedDistro } else { [string]$state.manager_distro }
        if ($distro -ne [string]$state.manager_distro) {
            throw 'Payload upgrade must target the journaled manager distribution.'
        }
        $running = @(Get-WslEgressRunningDistributions)
        if ($running -notcontains $distro) {
            throw 'Start the manager distribution before the reviewed payload upgrade.'
        }
        Install-WslEgressPayload $state $distro -ReviewedUpgrade
        $repair = Invoke-WslEgressLinux $state 'repair-config'
        if ($repair.outcome -ne 'succeeded') {
            throw 'Manager repair-config did not succeed after the payload upgrade.'
        }
        $audit = Invoke-WslEgressLinux $state 'audit-config'
        if ($audit.outcome -ne 'succeeded' -or
            -not $audit.facts.owned_config_matches -or
            -not $audit.facts.interop_runtime_disabled) {
            throw 'Post-upgrade owned-config audit failed.'
        }
        $state.phase = 'installed_closed'
        Save-WslEgressJson $state (Join-Path $script:StateRoot 'state.json')
        $result = [ordered]@{
            schema_version = 1
            action = 'update-live-payload'
            outcome = 'succeeded'
            generation = [long]$state.generation
            distro = $distro
            phase = 'installed_closed'
            owned_config_matches = [bool]$audit.facts.owned_config_matches
            interop_runtime_disabled = [bool]$audit.facts.interop_runtime_disabled
            endpoint_enabled = $false
            message = 'Reviewed payload and owned WSL configuration upgraded while the endpoint remained closed.'
        }
        $receipt = Join-Path $env:LOCALAPPDATA ('ClashOfRust\WslEgress\results\payload-upgrade-' +
            (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N') + '.json')
        Save-WslEgressJson $result $receipt
        $result['receipt'] = $receipt
        $result
    } $Distro
    $result | ConvertTo-Json -Depth 8 -Compress
} catch {
    [ordered]@{
        schema_version = 1
        action = 'update-live-payload'
        outcome = 'failed'
        message = 'Payload upgrade failed; endpoint remains closed and the journal is retained for review.'
    } | ConvertTo-Json -Compress
    exit 1
}
