#Requires -Version 5.1
# Offline tests: replace every host/WSL/network boundary before invoking actions.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$module = Import-Module (Join-Path $PSScriptRoot '../WslSecureEgress.psm1') -Force -PassThru
& $module {
    $script:Calls = [Collections.Generic.List[string]]::new()
    $script:MockAdministrator = $false
    $script:MockState = $null
    function script:Assert-WslEgressUser { 'S-1-5-21-offline-fixture' }
    function script:Test-WslEgressAdministrator { $script:MockAdministrator }
    function script:Get-WslEgressDistributions {
        $script:Calls.Add('enumerate')
        @([pscustomobject]@{ name = 'Ubuntu'; state = 'stopped'; coverage = 'not_enrolled' })
    }
    function script:Read-WslEgressState { $script:MockState }
    function script:Get-WslEgressBaseline { throw 'Unexpected network request in offline test.' }
    function script:Invoke-WslEgressProcess { throw 'Unexpected WSL process in offline test.' }
    function script:Save-WslEgressJson { throw 'Unexpected state write in offline test.' }
    function script:Set-WslEgressPersistentGuard { throw 'Unexpected firewall mutation in offline test.' }
    function script:Set-WslEgressEndpoint { throw 'Unexpected endpoint mutation in offline test.' }

    $count = 0
    function Assert-Equal($Actual, $Expected, [string]$Label) {
        if ($Actual -cne $Expected) { throw "FAIL: $Label" }
        $script:TestCount++
    }
    $script:TestCount = 0
    $doctor = Invoke-WslEgressOperation -Action Doctor
    Assert-Equal $doctor.outcome 'succeeded' 'doctor works without administrator'
    Assert-Equal $doctor.protection 'unknown' 'uninstalled does not claim protection'
    Assert-Equal $doctor.generation 0 'uninstalled has no old generation'
    Assert-Equal $doctor.distributions[0].state 'unknown' 'doctor does not infer running state'
    foreach ($action in @('Install','Start','Stop','Rollback','Uninstall')) {
        $preview = Invoke-WslEgressOperation -Action $action
        Assert-Equal $preview.outcome 'not_run' "preview $action cannot mutate"
        Assert-Equal $preview.phase 'preflight' "preview $action reports preflight"
        Assert-Equal $preview.protection 'unknown' "preview $action cannot claim a block"
    }
    $blocked = Invoke-WslEgressOperation -Action Install -Backend WireGuard -Execute
    Assert-Equal $blocked.outcome 'blocked' 'nonadministrator install fails before mutation'
    $script:MockAdministrator = $true
    $blocked = Invoke-WslEgressOperation -Action Install -Backend Tailscale -Execute
    Assert-Equal $blocked.outcome 'blocked' 'strict Tailscale fails before widening permissions'
    Assert-Equal $blocked.protection 'unknown' 'blocked request is not a proven packet block'
    $test = Invoke-WslEgressOperation -Action Test
    Assert-Equal $test.outcome 'not_run' 'acceptance not invented'
    Assert-Equal $test.phase 'acceptance_pending' 'acceptance pending is distinct from connection'
    Assert-Equal @($test.checks | Where-Object { $_.result -ne 'NOT_RUN' }).Count 0 'no unexecuted runtime test is passed'
    Assert-Equal ($script:Calls | Where-Object { $_ -ne 'enumerate' } | Measure-Object).Count 0 'no boundary calls escaped mocks'
    $base = @(
        [pscustomobject]@{host='portal.example.test';dns='PASS';tls='PASS';http_status=302;expected_statuses=@(200,302)},
        [pscustomobject]@{host='api.example.test';dns='PASS';tls='PASS';http_status=200;expected_statuses=@(200)}
    )
    Assert-WslEgressBaseline $base
    $script:TestCount++
    foreach ($field in @('dns','tls','http_status')) {
        $bad = $base | ConvertTo-Json | ConvertFrom-Json
        if ($field -eq 'http_status') { $bad[1].$field = 503 } else { $bad[1].$field = 'FAIL' }
        $rejected = $false
        try { Assert-WslEgressBaseline $bad } catch { $rejected = $true }
        Assert-Equal $rejected $true "bad $field baseline prevents changes"
    }
    $response = New-WslEgressResponse -Action Status -Outcome not_run
    $json = $response | ConvertTo-Json -Compress
    Assert-Equal ($json.Contains('endpoint_ipv4')) $false 'public status excludes endpoint'
    Assert-Equal ($json.Contains('private_key')) $false 'public status excludes credentials'
    Assert-Equal $response.schema_version 1 'response schema version'
    $linuxFixture = [pscustomobject]@{
        schema = 1; schema_version = 1; generation = 7; action = 'audit-config'
        outcome = 'succeeded'; protection = 'Unverified'; phase = 'Running'
        backend = 'wireguard'; message = 'fixture'
        checks = @([pscustomobject]@{ name = 'owned-config'; outcome = 'Passed' })
        facts = [pscustomobject]@{ owned_config_matches = $true; interop_runtime_disabled = $true; shared_netns = 'net:[1]' }
    }
    $converted = ConvertFrom-WslEgressLinuxEvent $linuxFixture 'audit-config' 7 'WireGuard' '00000000-0000-0000-0000-000000000001'
    Assert-Equal $converted.schema_version 1 'Linux event translates to the desktop schema'
    Assert-Equal $converted.operation_id '00000000-0000-0000-0000-000000000001' 'Linux event receives operation identity'
    Assert-Equal $converted.action 'audit-config' 'Linux event preserves action'
    Assert-Equal $converted.checks[0].id 'owned-config' 'Linux check names translate'
    Assert-Equal $converted.checks[0].result 'PASSED' 'Linux check outcomes translate'
    Assert-Equal $converted.facts.shared_netns 'net:[1]' 'Linux audit facts remain available to Windows'
    $profileState = [ordered]@{ endpoint_ipv4 = '203.0.113.8'; endpoint_port = 51820; manager_distro = 'COR-Egress' }
    $profile = [pscustomobject]@{ endpoint_ipv4 = '203.0.113.8'; endpoint_port = 51820; manager_distro = 'COR-Egress' }
    Assert-WslEgressProfileForState $profile $profileState
    $script:TestCount++
    $profile.endpoint_ipv4 = '203.0.113.9'
    $rejected = $false
    try { Assert-WslEgressProfileForState $profile $profileState } catch { $rejected = $true }
    Assert-Equal $rejected $true 'start refuses a profile that differs from the installed endpoint'
    Assert-Equal (New-WslEgressUnixPath 'C:\Private\egress.json') '/mnt/c/Private/egress.json' 'profile path maps to WSL mount path'
    $old = "[wsl2]`n# keep memory preference`nmemory=8GB`n[experimental]`nnetworkingMode=mirrored # legacy`nautoProxy=true`ndnsTunneling=true`nignoredPorts=3000`n"
    $updated = ConvertTo-WslEgressConfig $old
    Assert-Equal ($updated.Contains('memory=8GB')) $true 'merge preserves unrelated values'
    Assert-Equal ($updated.Contains('# keep memory preference')) $true 'merge preserves comments'
    Assert-Equal ($updated.Contains('ignoredPorts=3000')) $true 'merge preserves unrelated experimental option'
    Assert-Equal ([regex]::Matches($updated,'(?im)^networkingMode=').Count) 1 'merge removes legacy duplicate'
    Assert-Equal ([regex]::Matches($updated,'(?im)^autoProxy=').Count) 1 'merge has one proxy setting'
    Assert-Equal ($updated.Contains('networkingMode=nat')) $true 'NAT declared'
    Assert-Equal ($updated.Contains('dnsTunneling=false')) $true 'no host DNS tunnel fallback'
    Assert-Equal ((Get-WslEgressConfigValue $updated 'wsl2' 'networkingMode')) 'nat' 'NAT is in wsl2'
    Assert-Equal ((Get-WslEgressConfigValue $updated 'wsl2' 'localhostForwarding')) 'false' 'localhost forwarding is in wsl2'
    Assert-Equal ((Get-WslEgressConfigValue $updated 'experimental' 'dnsTunneling')) 'false' 'DNS tunneling is experimental'
    Assert-Equal ((Get-WslEgressConfigValue $updated 'experimental' 'autoProxy')) 'false' 'auto proxy is experimental'
    Assert-Equal ((Get-WslEgressConfigValue $updated 'experimental' 'firewall')) 'true' 'firewall is experimental'
    Assert-Equal ((Get-WslEgressConfigValue $updated 'wsl2' 'dnsTunneling')) $null 'DNS tunneling is not in wsl2'
    Assert-Equal (ConvertTo-WslEgressConfig $updated) (ConvertTo-WslEgressConfig (ConvertTo-WslEgressConfig $updated)) 'merge idempotent'
    [pscustomobject]@{ suite = 'windows-offline'; result = 'PASS'; assertions = $script:TestCount
        network_tests = 'NOT_RUN'; wsl_started = $false; firewall_modified = $false } | ConvertTo-Json -Compress
}
