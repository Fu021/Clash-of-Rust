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
    function script:Start-WslEgressLifetime { throw 'Unexpected WSL lifetime process in offline test.' }

    $count = 0
    function Assert-Equal($Actual, $Expected, [string]$Label) {
        if ($Actual -cne $Expected) { throw "FAIL: $Label" }
        $script:TestCount++
    }
    $script:TestCount = 0
    $connections = @(ConvertFrom-WslEgressConnections '[{"url":"https://portal.example.test/","accepted_statuses":[200,302]},{"url":"https://api.example.test/","accepted_statuses":[200]}]')
    Assert-Equal $connections.Count 2 'JSON baseline array is enumerated under Windows PowerShell 5.1'
    Assert-Equal ([uri]$connections[0].url).Host 'portal.example.test' 'each baseline entry has one URI'
    Assert-Equal $connections[1].accepted_statuses.Count 1 'accepted status list remains attached to its own URI'
    Assert-Equal (ConvertTo-WslEgressProcessArgument '--distribution') '--distribution' 'WSL options remain unquoted'
    Assert-Equal (ConvertTo-WslEgressProcessArgument 'Ubuntu-22.04') 'Ubuntu-22.04' 'simple WSL argument remains unquoted'
    Assert-Equal (ConvertTo-WslEgressProcessArgument 'a b') '"a b"' 'spaced argument is quoted'
    $roundtrip = ConvertTo-WslEgressOrderedState ('{"empty":[],"single":["one"],"multiple":["one","two"],"map":{}}' | ConvertFrom-Json)
    Assert-Equal ($roundtrip.empty -is [Array]) $true 'empty journal arrays retain array identity'
    Assert-Equal $roundtrip.empty.Count 0 'empty journal array has no synthetic dictionary item'
    Assert-Equal ($roundtrip.single -is [Array]) $true 'single-item journal arrays retain array identity'
    Assert-Equal $roundtrip.single.Count 1 'single-item journal array has one item'
    Assert-Equal $roundtrip.multiple.Count 2 'multi-item journal array retains both items'
    Assert-Equal ($roundtrip.map -is [Collections.IDictionary]) $true 'JSON maps remain dictionaries'
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
    $streamRejected = $false
    try { $null = New-WslEgressUnixPath 'C:\Private\egress.json:ads' } catch { $streamRejected = $true }
    Assert-Equal $streamRejected $true 'an alternate-stream profile path is refused'
    $networkRejected = $false
    try { $null = New-WslEgressUnixPath '\\server\share\egress.json' } catch { $networkRejected = $true }
    Assert-Equal $networkRejected $true 'a network profile path is refused'
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
    function script:Get-WslEgressSettings {
        [ordered]@{Enabled='True';DefaultInboundAction='Block';DefaultOutboundAction='Block';AllowHostPolicyMerge='False';LoopbackEnabled='False'}
    }
    $script:RuleFixture = [pscustomobject]@{
        Name='ClashOfRust-WslEgress-Endpoint';Enabled='True';Direction='Outbound';Action='Allow'
        VMCreatorId='{40E0AC32-46A5-438A-A0B2-2B479E8F2E90}'
        RemoteAddresses='203.0.113.8';RemotePorts='51820';Protocol='UDP'
    }
    function script:Get-NetFirewallHyperVRule { $script:RuleFixture }
    $auditState = [ordered]@{endpoint_ipv4='203.0.113.8';endpoint_port=51820;endpoint_enabled=$true}
    Assert-Equal (Get-WslEgressGuardAudit $auditState).valid $true 'Hyper-V scalar address and port audit matches the complete tuple'
    $script:RuleFixture.RemoteAddresses = @('203.0.113.8')
    $script:RuleFixture.RemotePorts = @('51820')
    Assert-Equal (Get-WslEgressGuardAudit $auditState).valid $true 'Hyper-V array address and port audit also matches'
    $script:RuleFixture.RemotePorts = '51821'
    Assert-Equal (Get-WslEgressGuardAudit $auditState).valid $false 'different scalar endpoint port is rejected'
    Assert-Equal (Get-WslEgressClosureAudit $null).confirmed $false 'absent state cannot establish endpoint closure'
    Assert-Equal (Get-WslEgressClosureAudit $auditState).confirmed $false 'enabled endpoint is not reported closed'
    $script:RuleFixture.Enabled = 'False'
    $auditState.endpoint_enabled = $false
    Assert-Equal (Get-WslEgressClosureAudit $auditState).confirmed $true 'disabled endpoint and effective deny establish static closure'
    function script:Get-WslEgressSettings { throw 'Fixture effective-policy read failure.' }
    Assert-Equal (Get-WslEgressClosureAudit $auditState).confirmed $false 'failed policy readback cannot claim closure'
    $upgradePath = Join-Path $PSScriptRoot '../Update-LiveWslPayload.ps1'
    $tokens = $null
    $errors = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile($upgradePath, [ref]$tokens, [ref]$errors)
    Assert-Equal @($errors).Count 0 'live payload upgrade script parses'
    $upgradeText = Get-Content -LiteralPath $upgradePath -Raw
    Assert-Equal ($upgradeText.Contains('repair-config')) $true 'live upgrade reruns the manager repair path'
    Assert-Equal ($upgradeText.Contains('-ReviewedUpgrade')) $true 'live upgrade uses the atomic reviewed upgrade gate'
    Assert-Equal ($upgradeText.Contains('Set-WslEgressEndpoint $state $true')) $false 'live upgrade never opens the endpoint'
    $uninstallCheckPath = Join-Path $PSScriptRoot '../Uninstall-Check.ps1'
    $uninstallTokens = $null
    $uninstallErrors = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile($uninstallCheckPath, [ref]$uninstallTokens, [ref]$uninstallErrors)
    Assert-Equal @($uninstallErrors).Count 0 'installer uninstall gate parses'
    # Uninstall must recognise the documented helper-package service location;
    # comparing against this script's own copy blocked every real uninstall.
    $programFiles = [Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFiles)
    $helperGuardian = Join-Path $programFiles 'Clash of Rust WSL\clash-wsl-guardian.exe'
    $appGuardian = Join-Path $programFiles 'Clash of Rust\clash-wsl-guardian.exe'
    Assert-Equal (Test-WslEgressGuardianServicePath ('"' + $helperGuardian + '" --service')) $true 'helper-package guardian service location is accepted'
    Assert-Equal (Test-WslEgressGuardianServicePath ('"' + $appGuardian + '" --service')) $true 'application-package guardian service location is accepted'
    Assert-Equal (Test-WslEgressGuardianServicePath ('"' + (Join-Path $PSScriptRoot '../../clash-wsl-guardian.exe') + '" --service')) $false 'guardian service outside Program Files is refused'
    Assert-Equal (Test-WslEgressGuardianServicePath ($helperGuardian + ' --service')) $false 'unquoted service command line is refused'
    Assert-Equal (Test-WslEgressGuardianServicePath ('"' + $helperGuardian + '"')) $false 'service command line without the service switch is refused'
    Assert-Equal (Test-WslEgressGuardianServicePath ('"' + (Join-Path $programFiles 'Clash of Rust WSL\clash-wsl-guardian-other.exe') + '" --service')) $false 'a different binary in the helper directory is refused'
    # Install must adopt a guardian service that another canonical package
    # already registered; both layouts share the fixed service name.
    $script:MockPackageBinary = $appGuardian
    $script:MockService = $null
    $script:MockCreatedBinary = $null
    $script:MockStarted = 0
    function script:Get-WslEgressGuardianPackageBinary { $script:MockPackageBinary }
    function script:Get-CimInstance { if ($script:MockService) { $script:MockService } }
    function script:New-Service {
        param($Name, $DisplayName, $BinaryPathName, $StartupType, $Description)
        $script:MockCreatedBinary = [string]$BinaryPathName
    }
    function script:Start-Service { param($Name) $script:MockStarted++ }
    Assert-Equal (Get-WslEgressGuardianServiceCommand $appGuardian) ('"' + $appGuardian + '" --service') 'the created service command line is the pinned form'
    Assert-Equal (Test-WslEgressGuardianServicePath (Get-WslEgressGuardianServiceCommand $appGuardian)) $true 'the created command line is an accepted canonical location'
    Assert-Equal (Test-WslEgressGuardianServiceIdentity ([pscustomobject]@{ PathName = ('"' + $helperGuardian + '" --service'); StartName = 'LocalSystem' })) $true 'a helper-package system service is owned'
    Assert-Equal (Test-WslEgressGuardianServiceIdentity ([pscustomobject]@{ PathName = ('"' + $appGuardian + '" --service'); StartName = 'NT AUTHORITY\SYSTEM' })) $true 'an application-package system service is owned'
    Assert-Equal (Test-WslEgressGuardianServiceIdentity ([pscustomobject]@{ PathName = ('"' + (Join-Path $PSScriptRoot '../../clash-wsl-guardian.exe') + '" --service'); StartName = 'LocalSystem' })) $false 'a service outside Program Files is not owned'
    Assert-Equal (Test-WslEgressGuardianServiceIdentity ([pscustomobject]@{ PathName = ('"' + $helperGuardian + '" --service'); StartName = 'NT AUTHORITY\LocalService' })) $false 'a non-system account is not owned'
    Assert-Equal (Test-WslEgressGuardianServiceIdentity $null) $false 'an absent service is not owned'
    $script:MockService = [pscustomobject]@{ PathName = ('"' + $helperGuardian + '" --service'); StartName = 'LocalSystem'; StartMode = 'Auto' }
    Install-WslEgressGuardian
    $script:TestCount++
    Assert-Equal $script:MockCreatedBinary $null 'an existing canonical service is adopted instead of recreated'
    Assert-Equal $script:MockStarted 1 'the adopted service is started'
    $script:MockService = [pscustomobject]@{ PathName = ('"' + (Join-Path $PSScriptRoot '../../clash-wsl-guardian.exe') + '" --service'); StartName = 'LocalSystem'; StartMode = 'Auto' }
    $script:MockStarted = 0
    $foreignRejected = $false
    try { Install-WslEgressGuardian } catch { $foreignRejected = $true }
    Assert-Equal $foreignRejected $true 'a foreign service image keeps installation closed'
    Assert-Equal $script:MockStarted 0 'a refused service is never started'
    [pscustomobject]@{ suite = 'windows-offline'; result = 'PASS'; assertions = $script:TestCount
        network_tests = 'NOT_RUN'; wsl_started = $false; firewall_modified = $false } | ConvertTo-Json -Compress
}
