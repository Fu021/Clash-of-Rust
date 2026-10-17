# Windows host operations are deliberately separate from the existing Mihomo
# service. Persistent Hyper-V policy is the guard; no GUI/watchdog opens it.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:Creator = '{40E0AC32-46A5-438A-A0B2-2B479E8F2E90}'
$script:Prefix = 'ClashOfRust-WslEgress-'
$script:StateRoot = Join-Path $env:ProgramData 'ClashOfRust\WslEgress'
$script:LinuxHelper = '/usr/local/lib/cor-egress/manager'

function New-WslEgressResponse {
    param([string]$Action, [string]$Backend = 'WireGuard',
        [string]$Outcome = 'not_run', [string]$Phase = 'uninstalled',
        [string]$Protection = 'unknown', [string]$Message = '', [long]$Generation = 0)
    [ordered]@{
        schema_version = 1; operation_id = [guid]::NewGuid().ToString('N')
        generation = $Generation; action = $Action; outcome = $Outcome
        protection = $Protection; phase = $Phase; backend = $Backend
        message = $Message; distributions = @(); checks = @(); logs = @()
    }
}

function ConvertFrom-WslEgressLinuxEvent {
    param([object]$Value, [string]$Action, [long]$Generation,
        [string]$Backend, [string]$OperationId)
    $checks = @()
    foreach ($check in @($Value.checks)) {
        $name = [string]$check.name
        $result = [string]$check.outcome
        if ($name) {
            $checks += [ordered]@{
                id = $name
                result = $result.ToUpperInvariant()
                detail = 'Linux helper local invariant result.'
            }
        }
    }
    $converted = [ordered]@{
        schema_version = 1
        operation_id = $OperationId
        generation = $Generation
        action = $Action
        outcome = $(if ([string]$Value.outcome -in @('Succeeded','succeeded')) { 'succeeded' } else { 'blocked' })
        protection = $(if ([string]$Value.protection -eq 'Blocked') { 'blocked' } else { 'unknown' })
        phase = [string]$Value.phase
        backend = $Backend
        message = [string]$Value.message
        distributions = @()
        checks = @($checks)
        logs = @()
    }
    if ($null -ne $Value.PSObject.Properties['facts']) {
        $converted['facts'] = $Value.facts
    }
    $converted
}

function New-WslEgressUnixPath([string]$Path) {
    if (-not $Path -or $Path.Length -gt 65536 -or $Path.IndexOf([char]0) -ge 0 -or
        $Path -notmatch '^[A-Za-z]:\\') { throw 'An absolute local Windows profile path is required.' }
    $full = [IO.Path]::GetFullPath($Path)
    # The drive colon sits at index 1 after the absolute-path check above. Any
    # further colon is an alternate data stream, which WSL cannot address.
    if ($full -match '^\\\\' -or $full.Substring(2).IndexOf(':') -ge 0) {
        throw 'Network and alternate-stream profile paths are unsupported.'
    }
    $relative = $full.Substring(3).Replace('\', '/')
    if ($relative -notmatch '^[^/]+(?:/[^/]+)*$') { throw 'Profile path cannot be represented safely in WSL.' }
    '/mnt/' + $full.Substring(0, 1).ToLowerInvariant() + '/' + $relative
}

function Assert-WslEgressProfileForState([object]$Profile, [object]$State) {
    if ([string]$Profile.endpoint_ipv4 -ne [string]$State.endpoint_ipv4 -or
        [int]$Profile.endpoint_port -ne [int]$State.endpoint_port -or
        [string]$Profile.manager_distro -ne [string]$State.manager_distro) {
        throw 'Private profile does not match the installed endpoint and manager distribution.'
    }
}

function Test-WslEgressAdministrator {
    $principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
    $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Assert-WslEgressUser {
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    if ($sid -in @('S-1-5-18','S-1-5-19','S-1-5-20')) {
        throw 'The per-user WSL broker cannot run under a service identity.'
    }
    $sid
}

function Get-WslEgressFileHash([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $null }
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
}

function Protect-WslEgressDirectory([string]$Path) {
    New-Item -ItemType Directory -Path $Path -Force | Out-Null
    $acl = New-Object Security.AccessControl.DirectorySecurity
    $acl.SetAccessRuleProtection($true, $false)
    foreach ($sid in @('S-1-5-18','S-1-5-32-544')) {
        $rule = New-Object Security.AccessControl.FileSystemAccessRule(
            (New-Object Security.Principal.SecurityIdentifier($sid)), 'FullControl',
            'ContainerInherit,ObjectInherit', 'None', 'Allow')
        $acl.AddAccessRule($rule)
    }
    $owner = New-Object Security.Principal.SecurityIdentifier('S-1-5-32-544')
    $acl.SetOwner($owner)
    $reader = New-Object Security.AccessControl.FileSystemAccessRule(
        ([Security.Principal.WindowsIdentity]::GetCurrent().User), 'ReadAndExecute',
        'ContainerInherit,ObjectInherit', 'None', 'Allow')
    $acl.AddAccessRule($reader)
    Set-Acl -LiteralPath $Path -AclObject $acl
}

function ConvertTo-WslEgressConfig([string]$Text) {
    $ownedBySection = @{
        wsl2 = @('networkingmode', 'localhostforwarding')
        experimental = @('dnstunneling', 'autoproxy', 'firewall')
    }
    $settingsBySection = [ordered]@{
        wsl2 = @('networkingMode=nat', 'localhostForwarding=false')
        experimental = @('dnsTunneling=false', 'autoProxy=false', 'firewall=true')
    }
    $owned = @($ownedBySection.Values | ForEach-Object { $_ }) | Select-Object -Unique
    $lines = New-Object 'Collections.Generic.List[string]'
    $section = ''
    $written = @{ wsl2 = $false; experimental = $false }
    function Add-WslEgressSectionSettings([string]$Name) {
        if ($Name -in $settingsBySection.Keys -and -not $written[$Name]) {
            foreach ($setting in $settingsBySection[$Name]) { $lines.Add($setting) }
            $written[$Name] = $true
        }
    }
    foreach ($line in ($Text.TrimEnd([char[]]"`r`n") -split '\r?\n')) {
        if ($line -match '^\s*\[([^]]+)\]') {
            Add-WslEgressSectionSettings $section
            $section = $Matches[1].ToLowerInvariant()
        }
        if ($section -in @('wsl2', 'experimental') -and $line -match '^\s*([^#;=]+?)\s*=') {
            if ($owned -contains $Matches[1].Trim().ToLowerInvariant()) { continue }
        }
        $lines.Add($line)
    }
    Add-WslEgressSectionSettings $section
    foreach ($targetSection in $settingsBySection.Keys) {
        if (-not $written[$targetSection]) {
            $lines.Add("[$targetSection]")
            foreach ($setting in $settingsBySection[$targetSection]) { $lines.Add($setting) }
        }
    }
    ($lines -join "`r`n") + "`r`n"
}

function Install-WslEgressConfig([object]$State) {
    $path = Join-Path $env:USERPROFILE '.wslconfig'
    $State.wslconfig = [ordered]@{ existed = (Test-Path -LiteralPath $path); before_hash = $null; after_hash = $null }
    $text = ''
    if ($State.wslconfig.existed) {
        if ((Get-Item -LiteralPath $path).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'WSL config reparse points require manual review.' }
        $State.wslconfig.before_hash = Get-WslEgressFileHash $path
        [IO.File]::WriteAllBytes((Join-Path $script:StateRoot 'wslconfig.before'), [IO.File]::ReadAllBytes($path))
        $text = [IO.File]::ReadAllText($path)
    }
    Save-WslEgressJson $State (Join-Path $script:StateRoot 'state.json')
    [IO.File]::WriteAllText($path, (ConvertTo-WslEgressConfig $text), (New-Object Text.UTF8Encoding($false)))
    $State.wslconfig.after_hash = Get-WslEgressFileHash $path
    Save-WslEgressJson $State (Join-Path $script:StateRoot 'state.json')
    Assert-WslEgressBaseline @(Get-WslEgressBaseline)
}

function Restore-WslEgressConfig([object]$State) {
    if (-not $State.PSObject.Properties['wslconfig'] -and $State -isnot [Collections.IDictionary]) { return }
    if (-not $State.wslconfig -or -not $State.wslconfig.after_hash) { return }
    $path = Join-Path $env:USERPROFILE '.wslconfig'
    if ((Get-WslEgressFileHash $path) -ne $State.wslconfig.after_hash) { throw 'WSL config changed after installation; targeted restore refused.' }
    if ($State.wslconfig.existed) {
        $snapshot = Join-Path $script:StateRoot 'wslconfig.before'
        if ((Get-WslEgressFileHash $snapshot) -ne $State.wslconfig.before_hash) { throw 'WSL config snapshot hash mismatch.' }
        [IO.File]::WriteAllBytes($path, [IO.File]::ReadAllBytes($snapshot))
    } else { Remove-Item -LiteralPath $path -ErrorAction Stop }
    Assert-WslEgressBaseline @(Get-WslEgressBaseline)
}

function Install-WslEgressPayload {
    param([object]$State, [string]$Distro, [switch]$ReviewedUpgrade)
    $runtimeNames = @('manager', 'manager.py', 'secure-egress', 'cor-egress-manager')
    $source = Join-Path $PSScriptRoot 'linux'
    $installer = Join-Path $source 'install-payload.py'
    if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) { throw 'Linux payload installer is missing.' }
    $files = @()
    foreach ($name in @($runtimeNames + 'install-payload.py')) {
        $path = Join-Path $source $name
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Linux payload file is missing: $name" }
        $bytes = [IO.File]::ReadAllBytes($path)
        if ($bytes.Length -gt 1048576 -or [Array]::IndexOf($bytes, [byte]13) -ge 0) {
            throw "Linux payload is oversized or not LF-normalized: $name"
        }
        $files += [ordered]@{
            name = $name
            content = [Convert]::ToBase64String($bytes)
            sha256 = (Get-WslEgressFileHash $path).ToLowerInvariant()
        }
    }
    if (@($files).Count -ne 5) { throw 'Linux payload must contain exactly five reviewed files.' }
    $stage = '/root/.cor-egress-stage-' + [guid]::NewGuid().ToString('N')
    # This fixed bootstrap only stages the reviewed payload. It never executes
    # payload code, changes networking, or accepts a caller-selected path.
    $bootstrap = @'
import base64,hashlib,json,os,pathlib,re,stat,sys,tempfile
stage=pathlib.Path(sys.argv[1])
root=pathlib.Path('/root')
for path in (root,stage):
    if path.is_symlink(): raise RuntimeError('unsafe staging path')
st=root.stat()
if st.st_uid!=0 or st.st_mode&0o022: raise RuntimeError('unsafe root directory')
if not re.fullmatch(r'/root/\.cor-egress-stage-[0-9a-f]{32}',str(stage)): raise RuntimeError('invalid staging path')
if stage.exists() or stage.is_symlink(): raise RuntimeError('staging path collision')
data=json.load(sys.stdin)
allowed={'manager','manager.py','secure-egress','cor-egress-manager','install-payload.py'}
records=data.get('files') if data.get('schema')==1 else None
if not isinstance(records,list) or len(records)!=len(allowed) or {x.get('name') for x in records}!=allowed:
    raise RuntimeError('payload manifest differs from reviewed runtime files')
decoded={}
for record in records:
    name=record['name']
    if not re.fullmatch(r'[a-z][a-z0-9_.-]+',name): raise RuntimeError('unsafe payload name')
    blob=base64.b64decode(record['content'],validate=True)
    if len(blob)>1048576 or b'\r' in blob or hashlib.sha256(blob).hexdigest()!=record['sha256']:
        raise RuntimeError('payload hash/format mismatch')
    decoded[name]=blob
stage.mkdir(mode=0o700)
for name,blob in decoded.items():
    target=stage/name
    fd=os.open(target,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o755 if name!='manager.py' else 0o644)
    with os.fdopen(fd,'wb') as stream:
        stream.write(blob); stream.flush(); os.fsync(stream.fileno())
manifest={name:hashlib.sha256(decoded[name]).hexdigest() for name in decoded if name!='install-payload.py'}
fd=os.open(stage/'sha256.json',os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
with os.fdopen(fd,'w') as stream:
    json.dump(manifest,stream,sort_keys=True); stream.write('\n'); stream.flush(); os.fsync(stream.fileno())
os.chmod(stage/'install-payload.py',0o755)
print(json.dumps({'schema':1,'schema_version':1,'action':'stage-payload','outcome':'succeeded','stage':str(stage)}))
'@
    $staged = Invoke-WslEgressProcess -Arguments @('--distribution',$Distro,'--user','root','--exec','/usr/bin/python3','-I','-c',$bootstrap,$stage) `
        -InputText (ConvertTo-Json -InputObject ([ordered]@{ schema = 1; files = @($files) }) -Depth 6 -Compress) -TimeoutSeconds 60
    if ($staged.ExitCode -ne 0) { throw 'Linux payload staging failed; no transport permission opened.' }
    try {
        $stageResult = $staged.Output | ConvertFrom-Json
        if ($stageResult.outcome -ne 'succeeded' -or $stageResult.stage -ne $stage) { throw 'Linux staging response mismatch.' }
        $installArguments = @('--distribution',$Distro,'--user','root','--exec',
            '/usr/bin/python3','-I',($stage + '/install-payload.py'),'--payload',$stage,'--manifest',($stage + '/sha256.json'))
        if ($ReviewedUpgrade) { $installArguments += '--reviewed-upgrade' }
        $install = Invoke-WslEgressProcess -Arguments $installArguments `
            -TimeoutSeconds 60
        if ($install.ExitCode -ne 0) { throw 'Hash-pinned Linux payload installation failed.' }
        $installResult = $install.Output | ConvertFrom-Json
        if ($installResult.outcome -ne 'succeeded' -or $installResult.action -ne 'install-payload') {
            throw 'Linux payload installer returned an invalid result.'
        }
    } finally {
        # Exact, non-recursive cleanup. A failed cleanup leaves a private stage
        # for diagnosis and does not weaken the persistent host deny.
        $cleanup = @'
import os,pathlib,sys
stage=pathlib.Path(sys.argv[1])
names=['manager','manager.py','secure-egress','cor-egress-manager','install-payload.py','sha256.json']
for name in names:
    path=stage/name
    if path.is_symlink(): raise RuntimeError('unsafe staged file')
    if path.exists(): path.unlink()
for entry in stage.iterdir(): raise RuntimeError('unexpected staged entry')
stage.rmdir()
'@
        $removed = Invoke-WslEgressProcess -Arguments @('--distribution',$Distro,'--user','root','--exec','/usr/bin/python3','-I','-c',$cleanup,$stage) -TimeoutSeconds 30
        if ($removed.ExitCode -ne 0) { throw 'Reviewed Linux staging directory could not be removed safely.' }
    }
    Assert-WslEgressBaseline @(Get-WslEgressBaseline)
}

function Save-WslEgressJson([object]$Value, [string]$Path) {
    $directory = [IO.Path]::GetDirectoryName($Path)
    if ($directory) { New-Item -ItemType Directory -Path $directory -Force | Out-Null }
    $temporary = $Path + '.' + [guid]::NewGuid().ToString('N') + '.tmp'
    [IO.File]::WriteAllText($temporary, ($Value | ConvertTo-Json -Depth 30),
        (New-Object Text.UTF8Encoding($false)))
    Move-Item -LiteralPath $temporary -Destination $Path -Force
}

function Read-WslEgressState {
    $path = Join-Path $script:StateRoot 'state.json'
    if (-not (Test-Path -LiteralPath $path)) { return $null }
    $state = ConvertTo-WslEgressOrderedState (Get-Content -LiteralPath $path -Raw | ConvertFrom-Json)
    if ($state.schema_version -ne 1 -or $state.owner -ne 'ClashOfRust.WslEgress') { throw 'Unknown state owner.' }
    # Older broker versions serialized an empty JSON array as {} on reread.
    # Recover only the known empty distro list; the scope audit still compares
    # it with the complete current set of ordinary distributions.
    if ($state.distro_configs -is [Collections.IDictionary] -and $state.distro_configs.Count -eq 0) {
        $state.distro_configs = @()
    }
    $state
}

function ConvertTo-WslEgressOrderedState([object]$Value) {
    if ($null -eq $Value) { return $null }
    if ($Value -is [Collections.IDictionary]) {
        $result = [ordered]@{}
        foreach ($key in $Value.Keys) { $result[[string]$key] = ConvertTo-WslEgressOrderedState $Value[$key] }
        return $result
    }
    if ($Value -is [Collections.IEnumerable] -and $Value -isnot [string]) {
        $items = New-Object 'Collections.Generic.List[object]'
        foreach ($item in $Value) { $items.Add((ConvertTo-WslEgressOrderedState $item)) }
        return ,($items.ToArray())
    }
    if ($Value -is [Management.Automation.PSCustomObject]) {
        $result = [ordered]@{}
        foreach ($property in $Value.PSObject.Properties) {
            $result[$property.Name] = ConvertTo-WslEgressOrderedState $property.Value
        }
        return $result
    }
    $Value
}

function ConvertTo-WslEgressProcessArgument([string]$Argument) {
    if ($Argument.IndexOf([char]0) -ge 0) { throw 'Invalid argument.' }
    # WSL parses its own options before using Windows argv rules for --exec.
    # Quoting an option such as "--list" makes WSL treat it as a Linux command.
    if ($Argument -and $Argument -notmatch '[\s"]') { return $Argument }
    '"' + ([regex]::Replace(([regex]::Replace($Argument, '(\\*)"', '$1$1\"')), '(\\+)$', '$1$1')) + '"'
}

function Invoke-WslEgressProcess {
    param([string[]]$Arguments, [string]$InputText, [int]$TimeoutSeconds = 45,
        [switch]$UnicodeOutput)
    $executable = Join-Path $env:SystemRoot 'System32\wsl.exe'
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = $executable; $start.UseShellExecute = $false
    $start.CreateNoWindow = $true; $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true; $start.RedirectStandardInput = $true
    # Windows PowerShell lacks ArgumentList. Quote each argument using the
    # CommandLineToArgvW backslash rules; never use cmd /c or a shell command.
    $quoted = foreach ($argument in $Arguments) { ConvertTo-WslEgressProcessArgument $argument }
    $start.Arguments = $quoted -join ' '
    if ($UnicodeOutput) { $start.StandardOutputEncoding = [Text.Encoding]::Unicode }
    else { $start.StandardOutputEncoding = New-Object Text.UTF8Encoding($false) }
    $start.StandardErrorEncoding = $start.StandardOutputEncoding
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $start
    try {
        if (-not $process.Start()) { throw 'WSL process did not start.' }
        $output = $process.StandardOutput.ReadToEndAsync()
        $errors = $process.StandardError.ReadToEndAsync()
        # .NET Framework (Windows PowerShell 5.1) has no
        # ProcessStartInfo.StandardInputEncoding. Write UTF-8 directly and close
        # the underlying pipe without the framework writer adding a preamble.
        if ($InputText) {
            $bytes = (New-Object Text.UTF8Encoding($false)).GetBytes($InputText)
            $process.StandardInput.BaseStream.Write($bytes, 0, $bytes.Length)
        }
        $process.StandardInput.BaseStream.Close()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            # Killing this Windows broker does NOT certify Linux task exit.
            $process.Kill()
            throw 'WSL operation timed out; protection must remain installed.'
        }
        [pscustomobject]@{ ExitCode = $process.ExitCode; Output = $output.GetAwaiter().GetResult() }
        $null = $errors.GetAwaiter().GetResult() # Never log stderr containing secrets.
    } finally { $process.Dispose() }
}

function Get-WslEgressDistributions {
    $registered = @()
    $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Lxss'
    if (Test-Path -LiteralPath $key) {
        $registered = @(Get-ChildItem -LiteralPath $key | ForEach-Object {
            $item = Get-ItemProperty -LiteralPath $_.PSPath
            [pscustomobject]@{ name = [string]$item.DistributionName; state = 'unknown'; coverage = 'not_enrolled' }
        })
    }
    $registered
}

function Get-WslEgressRunningDistributions {
    $running = Invoke-WslEgressProcess -Arguments @('--list','--running','--quiet') -UnicodeOutput
    if ($running.ExitCode -ne 0) { throw 'WSL running-state enumeration failed.' }
    @($running.Output -split '[\r\n]+' | ForEach-Object { $_.Trim([char]0).Trim() } | Where-Object { $_ })
}

function ConvertFrom-WslEgressConnections([string]$Text) {
    $connections = ConvertFrom-Json -InputObject $Text
    foreach ($connection in $connections) { $connection }
}

function Get-WslEgressBaseline {
    $results = @()
    # Availability obligations are private deployment data, not upstream defaults.
    $path = Join-Path $env:LOCALAPPDATA 'ClashOfRust\WslEgress\protected-connections.json'
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw 'Configure the private mandatory-connection baseline before network deployment.' }
    # Windows PowerShell 5.1 emits a JSON array as one pipeline object. Assign
    # first, then enumerate the value, so each URL is checked independently.
    $connections = @(ConvertFrom-WslEgressConnections ([IO.File]::ReadAllText($path)))
    if ($connections.Count -eq 0 -or $connections.Count -gt 16) { throw 'Invalid mandatory connection list.' }
    foreach ($connection in $connections) {
        $uri = [uri]$connection.url
        if (-not $uri.IsAbsoluteUri -or $uri.Scheme -ne 'https' -or $uri.UserInfo -or $uri.Query -or $uri.Fragment -or
            @($connection.accepted_statuses).Count -eq 0) { throw 'Baseline must use credential-free HTTPS and explicit accepted HTTP statuses.' }
        $hostName = $uri.DnsSafeHost
        $entry = [ordered]@{ host = $hostName; observed_at = (Get-Date -Format o)
            expected_statuses = @($connection.accepted_statuses)
            dns = 'FAIL'; tls = 'NOT_RUN'; http_status = 0; path = 'Windows default proxy; certificate validation enabled' }
        try {
            $null = Resolve-DnsName -Name $hostName -DnsOnly -ErrorAction Stop
            $entry.dns = 'PASS'
            Add-Type -AssemblyName System.Net.Http
            $handler = New-Object Net.Http.HttpClientHandler
            $handler.AllowAutoRedirect = $false
            $client = New-Object Net.Http.HttpClient($handler)
            $client.Timeout = [TimeSpan]::FromSeconds(20)
            try {
                $response = $client.GetAsync($uri,
                    [Net.Http.HttpCompletionOption]::ResponseHeadersRead).GetAwaiter().GetResult()
                $entry.tls = 'PASS'; $entry.http_status = [int]$response.StatusCode
                $response.Dispose()
            } finally { $client.Dispose(); $handler.Dispose() }
        } catch { $entry.tls = 'FAIL' }
        $results += [pscustomobject]$entry
    }
    $results
}

function Assert-WslEgressBaseline([object[]]$Baseline) {
    if ($Baseline.Count -eq 0) { throw 'Mandatory connection results are absent.' }
    foreach ($entry in $Baseline) {
        if ($entry.dns -ne 'PASS' -or $entry.tls -ne 'PASS' -or
            $entry.http_status -notin @($entry.expected_statuses)) {
            throw 'A mandatory Windows connection is unhealthy. Further network changes are blocked.'
        }
    }
}

function Read-WslEgressProfile([string]$Path) {
    if (-not $Path -or -not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw 'A private profile is required.' }
    $item = Get-Item -LiteralPath $Path
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Profile reparse points are not accepted.' }
    if ($item.Length -gt 65536) { throw 'Profile is too large.' }
    $profile = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
    if (-not $profile.PSObject.Properties['schema'] -or $profile.schema -ne 1) { throw 'Profile schema must be 1.' }
    $address = $null
    if (-not [Net.IPAddress]::TryParse([string]$profile.endpoint_ipv4, [ref]$address) -or
        $address.AddressFamily -ne [Net.Sockets.AddressFamily]::InterNetwork -or
        [string]$address -ne [string]$profile.endpoint_ipv4 -or
        [Net.IPAddress]::IsLoopback($address) -or $address.GetAddressBytes()[0] -ge 224 -or
        $address.GetAddressBytes()[0] -eq 0) { throw 'An explicit unicast IPv4 endpoint is required.' }
    if ([int]$profile.endpoint_port -lt 1 -or [int]$profile.endpoint_port -gt 65535) { throw 'Invalid endpoint port.' }
    if ([string]$profile.manager_distro -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$') { throw 'Invalid manager distro.' }
    if ($profile.backend -notin @('WireGuard','Tailscale')) { throw 'Unknown backend.' }
    if ($profile.backend -eq 'WireGuard') {
        foreach ($keyName in @('private_key','peer_public_key')) {
            if ([Convert]::FromBase64String([string]$profile.$keyName).Length -ne 32) { throw 'WireGuard key length is invalid.' }
        }
        if (-not $profile.PSObject.Properties['tunnel_address'] -or
            -not $profile.PSObject.Properties['resolver_ipv4'] -or
            -not $profile.PSObject.Properties['uid'] -or -not $profile.PSObject.Properties['gid']) { throw 'Incomplete Linux profile.' }
    }
    $profile
}

function Get-WslEgressSettings {
    Get-WslEgressSettingsFromStore -PolicyStore ActiveStore
}

function Get-WslEgressSettingsFromStore([string]$PolicyStore) {
    $setting = Get-NetFirewallHyperVVMSetting -PolicyStore $PolicyStore -Name $script:Creator -ErrorAction Stop
    [ordered]@{
        Enabled = $setting.Enabled
        DefaultInboundAction = $setting.DefaultInboundAction
        DefaultOutboundAction = $setting.DefaultOutboundAction
        AllowHostPolicyMerge = $setting.AllowHostPolicyMerge
        LoopbackEnabled = $setting.LoopbackEnabled
    }
}

function Get-WslEgressPersistentSettings {
    Get-WslEgressSettingsFromStore -PolicyStore PersistentStore
}

function Get-WslEgressGuardAudit([object]$State) {
    $issues = @()
    $setting = Get-WslEgressSettings
    if ([string]$setting.Enabled -ne 'True' -or [string]$setting.DefaultOutboundAction -ne 'Block' -or
        [string]$setting.DefaultInboundAction -ne 'Block' -or [string]$setting.AllowHostPolicyMerge -ne 'False' -or
        [string]$setting.LoopbackEnabled -ne 'False') { $issues += 'effective_settings_differ' }
    $allows = @(Get-NetFirewallHyperVRule -PolicyStore ActiveStore -ErrorAction Stop | Where-Object {
        [string]$_.Enabled -eq 'True' -and [string]$_.Direction -eq 'Outbound' -and
        [string]$_.Action -eq 'Allow' -and [string]$_.VMCreatorId -eq $script:Creator
    })
    foreach ($rule in $allows) {
        if ($rule.Name -ne ($script:Prefix + 'Endpoint')) { $issues += 'foreign_outbound_allow'; continue }
        if (@($rule.RemoteAddresses).Count -ne 1 -or
            [string]@($rule.RemoteAddresses)[0] -notin @([string]$State.endpoint_ipv4, ([string]$State.endpoint_ipv4 + '/32')) -or
            [string]$rule.Protocol -notin @('UDP','17') -or
            @($rule.RemotePorts).Count -ne 1 -or [string]@($rule.RemotePorts)[0] -ne [string]$State.endpoint_port) {
            $issues += 'endpoint_rule_differ'
        }
    }
    if ($State.endpoint_enabled -and @($allows | Where-Object { $_.Name -eq ($script:Prefix + 'Endpoint') }).Count -ne 1) {
        $issues += 'endpoint_rule_missing'
    }
    if (-not $State.endpoint_enabled -and @($allows | Where-Object { $_.Name -eq ($script:Prefix + 'Endpoint') }).Count -ne 0) {
        $issues += 'endpoint_enabled_while_closed'
    }
    [pscustomobject]@{ valid = ($issues.Count -eq 0); issues = @($issues | Select-Object -Unique) }
}

function Invoke-WslEgressGuardianAudit {
    # This fixed audit is the only function used under SYSTEM. It neither
    # repairs external policy nor launches wsl.exe in the wrong user's context.
    $state = Read-WslEgressState
    if (-not $state -or $state.phase -eq 'restored') { return }
    $audit = Get-WslEgressGuardAudit $state
    $record = [ordered]@{ schema_version = 1; observed_at = (Get-Date -Format o)
        generation = $state.generation; policy_matches = $audit.valid
        issues = @($audit.issues); runtime_acceptance = 'NOT_RUN' }
    Save-WslEgressJson $record (Join-Path $script:StateRoot 'guardian-audit.json')
}

function Get-WslEgressClosureAudit([object]$State) {
    $confirmed = $false
    if ($State) {
        try { $confirmed = -not $State.endpoint_enabled -and (Get-WslEgressGuardAudit $State).valid }
        catch { $confirmed = $false }
    }
    [pscustomobject]@{ confirmed = [bool]$confirmed }
}

function Get-WslEgressGuardianServiceCommand([string]$Binary) { '"' + $Binary + '" --service' }

function Get-WslEgressGuardianPackageBinary {
    # The service image must come from the protected package that ships next to
    # this module; a checkout, user directory or junction is never acceptable.
    $binary = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../clash-wsl-guardian.exe'))
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw 'The packaged WSL guardian binary is missing.' }
    $programFiles = [Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFiles)
    if (-not $binary.StartsWith($programFiles.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw 'A service may only be installed from the protected Program Files package.'
    }
    $binary
}

function Test-WslEgressGuardianServiceIdentity([object]$Service) {
    # The helper package and the application package are both canonical Program
    # Files layouts, so a service created by either copy is owned: rewriting the
    # image of a live service is riskier than running the other audited copy,
    # and each canonical copy pins the resources of its own package. Only a
    # foreign account or an image outside Program Files is refused.
    if (-not $Service) { return $false }
    if ([string]$Service.StartName -notin @('LocalSystem', 'NT AUTHORITY\SYSTEM')) { return $false }
    return (Test-WslEgressGuardianServicePath ([string]$Service.PathName))
}

function Install-WslEgressGuardian {
    $expected = Get-WslEgressGuardianServiceCommand (Get-WslEgressGuardianPackageBinary)
    $service = Get-CimInstance Win32_Service -Filter "Name='ClashOfRust.WslEgress'"
    if ($service) {
        if (-not (Test-WslEgressGuardianServiceIdentity $service)) {
            throw 'WSL guardian service identity differs from the owned service.'
        }
    } else {
        New-Service -Name 'ClashOfRust.WslEgress' -DisplayName 'Clash of Rust WSL Egress Audit' `
            -BinaryPathName $expected -StartupType Automatic -Description 'Audits persistent WSL egress policy; never opens the guard or launches user WSL.' | Out-Null
    }
    # Cold-boot enforcement needs a persisted automatic start. An adopted service
    # that already reports Auto is left untouched and verified by the readback
    # below; anything else is reconfigured and re-read.
    if (-not $service -or [string]$service.StartMode -ne 'Auto') {
        & (Join-Path $env:SystemRoot 'System32\sc.exe') config 'ClashOfRust.WslEgress' start= auto | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'The WSL guardian service startup mode could not be persisted.' }
        $service = Get-CimInstance Win32_Service -Filter "Name='ClashOfRust.WslEgress'"
    }
    if (-not $service -or [string]$service.StartMode -ne 'Auto') {
        throw 'The WSL guardian service is not configured to start automatically.'
    }
    Start-Service -Name 'ClashOfRust.WslEgress' -ErrorAction Stop
}

function Test-WslEgressGuardianServicePath([string]$PathName) {
    # The guardian service may only run from a protected package directory. The
    # reviewed helper package normally installs it, but an application package
    # installed under Program Files is equally canonical. Accept exactly those
    # two locations so a foreign or relocated service is never removed.
    $programFiles = [Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFiles)
    if (-not $programFiles) { return $false }
    $match = [regex]::Match([string]$PathName, '^"([^"]+)" --service$')
    if (-not $match.Success) { return $false }
    try { $binary = [IO.Path]::GetFullPath($match.Groups[1].Value) } catch { return $false }
    foreach ($root in @('Clash of Rust WSL', 'Clash of Rust')) {
        $expected = [IO.Path]::GetFullPath((Join-Path $programFiles ($root + '\clash-wsl-guardian.exe')))
        if ($binary -ieq $expected) { return $true }
    }
    return $false
}

function Get-WslEgressDockerPaths {
    $candidates = @((Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFiles)) 'Docker\Docker\resources\com.docker.backend.exe'))
    foreach ($key in @('HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Docker Desktop',
        'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Docker Desktop')) {
        if (Test-Path -LiteralPath $key) {
            $item = Get-ItemProperty -LiteralPath $key
            if ($item.PSObject.Properties['InstallLocation'] -and $item.InstallLocation) {
                $candidates += Join-Path $item.InstallLocation 'resources\com.docker.backend.exe'
            }
        }
    }
    foreach ($process in @(Get-CimInstance Win32_Process -Filter "Name='com.docker.backend.exe'")) {
        if (-not $process.ExecutablePath) { throw 'Docker backend identity cannot be inspected.' }
        $candidates += [string]$process.ExecutablePath
    }
    @($candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | ForEach-Object {
        $path = [IO.Path]::GetFullPath($_)
        $current = $path
        while ($current) {
            if ((Get-Item -LiteralPath $current).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Docker executable uses a redirected path.' }
            $current = [IO.Path]::GetDirectoryName($current)
        }
        $path
    } | Select-Object -Unique)
}

function Get-WslEgressDockerContainerProcesses {
    $backend = Get-CimInstance Win32_Process -Filter "Name='com.docker.backend.exe'"
    if (-not $backend) { return @() }
    $ids = @($backend.ProcessId)
    $descendants = [Collections.Generic.HashSet[uint32]]::new()
    foreach ($id in $ids) { $null = $descendants.Add([uint32]$id) }
    # Docker backend containers can be grandchildren (backend -> helper ->
    # container shim). Traverse the complete descendant tree without accepting
    # arbitrary user processes.
    do {
        $added = $false
        foreach ($process in @(Get-CimInstance Win32_Process)) {
            if ($descendants.Contains([uint32]$process.ParentProcessId) -and
                $descendants.Add([uint32]$process.ProcessId)) { $added = $true }
        }
    } while ($added)
    @(Get-CimInstance Win32_Process | Where-Object { $descendants.Contains([uint32]$_.ProcessId) } |
        ForEach-Object { [string]$_.ExecutablePath } | Where-Object { $_ } | Select-Object -Unique)
}

function Import-WslEgressDockerGuard {
    if (-not ('CorDockerEgressGuard' -as [type])) { Add-Type -Path (Join-Path $PSScriptRoot 'DockerEgressGuard.cs') }
    [CorDockerEgressGuard]::ValidateLayout()
}

function Set-WslEgressDockerGuard([object]$State, [bool]$Remove = $false) {
    $paths = @($State.docker_backends)
    if (-not $Remove) {
        $paths += @(Get-WslEgressDockerContainerProcesses)
        $paths = @($paths | Where-Object { $_ } | Select-Object -Unique)
        $State.docker_backends = @($paths)
        Save-WslEgressJson $State (Join-Path $script:StateRoot 'state.json')
    }
    if ($paths.Count -eq 0) { return }
    if ($Remove -and @(Get-Process -Name 'com.docker.backend' -ErrorAction SilentlyContinue).Count -gt 0) {
        throw 'Stop Docker Desktop before restoring its external network.'
    }
    Import-WslEgressDockerGuard
    if ($Remove) {
        # The runtime descendant set is intentionally not durable. Remove both
        # the recorded identities and any current descendants using the same
        # exact path, then verify that the owned provider is empty.
        $removePaths = @($paths + @(Get-WslEgressDockerContainerProcesses) |
            Where-Object { $_ } | Select-Object -Unique)
        [CorDockerEgressGuard]::Apply([string[]]$removePaths, $true)
    } else {
        [CorDockerEgressGuard]::Apply([string[]]$paths, $false)
        if (-not [CorDockerEgressGuard]::Verify([string[]]$paths)) { throw 'Docker WFP readback failed.' }
    }
    Assert-WslEgressBaseline @(Get-WslEgressBaseline)
}

function Invoke-WslEgressLinux {
    param([object]$State, [string]$Action, [string]$InputText,
        [string[]]$AdditionalArguments = @(), [string]$Distribution = '',
        [string]$OperationId = '', [string]$LinuxProfilePath = '/etc/cor-egress/profile.json')
    $arguments = @('--distribution',[string]$State.manager_distro,'--user','root','--exec',
        $script:LinuxHelper,$Action,'--profile',$LinuxProfilePath,
        '--generation',[string]$State.generation)
    if ($Distribution) { $arguments[1] = $Distribution }
    $arguments += @($AdditionalArguments)
    $result = Invoke-WslEgressProcess -Arguments $arguments -InputText $InputText -TimeoutSeconds 60
    if ($result.Output.Length -gt 1048576) { throw 'Oversized Linux response.' }
    try { $response = $result.Output | ConvertFrom-Json } catch {
        $response = [pscustomobject]@{
            schema = 1; generation = [long]$State.generation; action = $Action
            outcome = 'blocked'; protection = 'Blocked'; phase = 'Blocked'
            message = 'Linux helper failed without a valid structured response.'
            checks = @()
        }
    }
    if ($response.schema -ne 1 -or $response.generation -ne $State.generation -or
        $response.action -ne $Action) { throw 'Linux response contract or generation mismatch.' }
    if (-not $OperationId) { $OperationId = [guid]::NewGuid().ToString() }
    $converted = ConvertFrom-WslEgressLinuxEvent $response $Action ([long]$State.generation) ([string]$State.backend) $OperationId
    if ($result.ExitCode -ne 0 -and $converted.outcome -ne 'blocked') {
        throw 'Linux helper exit status conflicts with its structured response.'
    }
    $converted
}

function Stop-WslEgressDistribution([string]$Name) {
    if (-not $Name -or $Name -match '[\r\n\0]') { throw 'Invalid distribution name.' }
    $result = Invoke-WslEgressProcess -Arguments @('--terminate',$Name) -TimeoutSeconds 45
    if ($result.ExitCode -ne 0) { throw 'WSL distribution termination failed.' }
}

function Get-WslEgressConfigValue([string]$Text, [string]$Section, [string]$Key) {
    $current = ''
    foreach ($line in ($Text -split '\r?\n')) {
        if ($line -match '^\s*\[([^]]+)\]\s*(?:[#;].*)?$') {
            $current = $Matches[1].Trim().ToLowerInvariant()
            continue
        }
        if ($current -eq $Section.ToLowerInvariant() -and
            $line -match '^\s*([^#;=]+?)\s*=\s*([^#;]*?)\s*(?:[#;].*)?$' -and
            $Matches[1].Trim().ToLowerInvariant() -eq $Key.ToLowerInvariant()) {
            return $Matches[2].Trim().ToLowerInvariant()
        }
    }
    $null
}

function Get-WslEgressWindowsConfigAudit([object]$State) {
    $path = Join-Path $env:USERPROFILE '.wslconfig'
    $currentHash = Get-WslEgressFileHash $path
    $expectedHash = $State.wslconfig.after_hash
    $text = if (Test-Path -LiteralPath $path -PathType Leaf) { [IO.File]::ReadAllText($path) } else { '' }
    $values = [ordered]@{
        networking_mode = Get-WslEgressConfigValue $text 'wsl2' 'networkingMode'
        dns_tunneling = Get-WslEgressConfigValue $text 'experimental' 'dnsTunneling'
        auto_proxy = Get-WslEgressConfigValue $text 'experimental' 'autoProxy'
        firewall = Get-WslEgressConfigValue $text 'experimental' 'firewall'
        localhost_forwarding = Get-WslEgressConfigValue $text 'wsl2' 'localhostForwarding'
    }
    $valid = $currentHash -eq $expectedHash -and
        $values.networking_mode -eq 'nat' -and $values.dns_tunneling -eq 'false' -and
        $values.auto_proxy -eq 'false' -and $values.firewall -eq 'true' -and
        $values.localhost_forwarding -eq 'false'
    [pscustomobject]@{ valid = $valid; hash_matches = $currentHash -eq $expectedHash; values = $values }
}

function Get-WslEgressOrdinaryDistributions([object[]]$Distributions) {
    @($Distributions | Where-Object { $_.name -notmatch '^docker-desktop(?:-data)?$' } |
        ForEach-Object { [string]$_.name })
}

function Get-WslEgressDockerBlocked([object]$State) {
    $current = @(Get-WslEgressDistributions)
    $dockerDistros = @($current | Where-Object { $_.name -match '^docker-desktop(?:-data)?$' })
    if ($dockerDistros.Count -eq 0) { return @($State.docker_backends).Count -eq 0 }
    $paths = @($State.docker_backends)
    try { $paths += @(Get-WslEgressDockerContainerProcesses) } catch { return $false }
    $paths = @($paths | Where-Object { $_ } | Select-Object -Unique)
    if ($paths.Count -eq 0) { return $false }
    Import-WslEgressDockerGuard
    foreach ($path in $paths) {
        try {
            if (-not [CorDockerEgressGuard]::Verify([string[]]@($path))) { return $false }
        } catch {
            return $false
        }
    }
    $true
}

function Get-WslEgressScopeAttestation([object]$State) {
    $distributions = @(Get-WslEgressDistributions)
    $currentNames = @($distributions | ForEach-Object { [string]$_.name })
    $recordedNames = @($State.distributions | ForEach-Object { [string]$_ })
    $sameScope = (@($currentNames | Sort-Object) -join "`n") -eq (@($recordedNames | Sort-Object) -join "`n")
    $configured = @($State.distro_configs)
    $expectedManaged = @($recordedNames | Where-Object {
        $_ -ne $State.manager_distro -and $_ -notmatch '^docker-desktop(?:-data)?$'
    } | Sort-Object)
    $allManaged = (@($configured | Sort-Object) -join "`n") -eq ($expectedManaged -join "`n") -and $sameScope
    $managerAudit = $null
    try { $managerAudit = Invoke-WslEgressLinux $State 'audit-config' }
    catch { $allManaged = $false }
    $netns = @()
    if ($managerAudit -and $managerAudit.facts.owned_config_matches -and
        $managerAudit.facts.interop_runtime_disabled) {
        $netns += [string]$managerAudit.facts.shared_netns
    } else { $allManaged = $false }
    foreach ($name in $configured) {
        try {
            $audit = Invoke-WslEgressLinux $State 'audit-config' -Distribution $name
            if (-not $audit.facts.owned_config_matches -or -not $audit.facts.interop_runtime_disabled) {
                $allManaged = $false
            } else { $netns += [string]$audit.facts.shared_netns }
        } catch { $allManaged = $false }
    }
    if ($allManaged -and (@($netns | Select-Object -Unique).Count -ne 1)) { $allManaged = $false }
    $hostAudit = Get-WslEgressGuardAudit $State
    $windowsConfig = Get-WslEgressWindowsConfigAudit $State
    $dockerBlocked = Get-WslEgressDockerBlocked $State
    $effective = $allManaged -and $hostAudit.valid -and $windowsConfig.valid -and $dockerBlocked
    [ordered]@{
        schema = 1
        generation = [long]$State.generation
        issued_at_unix = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
        all_distros_managed = [bool]$allManaged
        hyperv_default_deny = [bool]$hostAudit.valid
        dns_tunneling_disabled = [bool]$windowsConfig.valid
        interop_disabled = [bool]($managerAudit -and $managerAudit.facts.interop_runtime_disabled)
        docker_desktop_blocked = [bool]$dockerBlocked
        effective_audit_passed = [bool]$effective
    }
}

function Invoke-WslEgressDistroConfiguration([object]$State, [object]$Profile, [object[]]$Distributions) {
    $ordinary = @(Get-WslEgressOrdinaryDistributions $Distributions)
    if ($ordinary -notcontains [string]$State.manager_distro) {
        throw 'The manager distribution disappeared during installation.'
    }
    $profileText = $Profile | ConvertTo-Json -Depth 12 -Compress
    $configured = @()
    foreach ($name in $ordinary) {
        if ($name -ne $State.manager_distro) {
            Install-WslEgressPayload $State $name
            $result = Invoke-WslEgressLinux $State 'configure' -InputText $profileText -Distribution $name
            if ($result.outcome -ne 'succeeded') { throw "Linux distro configuration failed: $name" }
            $configured += $name
            Stop-WslEgressDistribution $name
        }
    }
    Stop-WslEgressDistribution ([string]$State.manager_distro)
    # The WSL VM can outlive distro termination. Reload global networking and
    # shared interop state after the complete set of distro files is ready.
    $shutdown = Invoke-WslEgressProcess -Arguments @('--shutdown') -TimeoutSeconds 45
    if ($shutdown.ExitCode -ne 0) { throw 'WSL VM shutdown failed; host guard remains closed.' }
    $State.distro_configs = @($configured)
    Save-WslEgressJson $State (Join-Path $script:StateRoot 'state.json')
    # Restart after all wsl.conf changes, then collect runtime facts. The host
    # default deny is already active, so this restart cannot open fallback egress.
    $managerAudit = Invoke-WslEgressLinux $State 'audit-config'
    if ($managerAudit.outcome -ne 'succeeded') { throw 'Runtime configuration audit failed: manager distribution.' }
    foreach ($name in @($configured)) {
        $audit = Invoke-WslEgressLinux $State 'audit-config' -Distribution $name
        if ($audit.outcome -ne 'succeeded') { throw "Runtime configuration audit failed: $name" }
    }
}

function Set-WslEgressPersistentGuard([object]$State) {
    # No blanket Block rule: it would override a later Endpoint Allow.
    Set-NetFirewallHyperVVMSetting -PolicyStore PersistentStore -Name $script:Creator `
        -Enabled True -DefaultInboundAction Block -DefaultOutboundAction Block `
        -AllowHostPolicyMerge False -LoopbackEnabled False -ErrorAction Stop
    $State.settings_written = $true
    Save-WslEgressJson $State (Join-Path $script:StateRoot 'state.json')
    $State.after_settings = Get-WslEgressSettings
    Save-WslEgressJson $State (Join-Path $script:StateRoot 'state.json')
    Assert-WslEgressBaseline @(Get-WslEgressBaseline)
    $audit = Get-WslEgressGuardAudit $State
    if (-not $audit.valid) { throw 'Effective host rules contain unowned allowances; no endpoint is opened.' }
}

function Set-WslEgressEndpoint([object]$State, [bool]$Enabled) {
    $existing = @(Get-NetFirewallHyperVRule -PolicyStore PersistentStore -Name ($script:Prefix + 'Endpoint') -ErrorAction SilentlyContinue)
    if ($existing.Count -gt 1) { throw 'Multiple owned endpoint rules exist; preserving them for manual review.' }
    if ($existing.Count -gt 0) {
        # Verify ownership against the exact tuple before changing an object.
        $rule = $existing[0]
        if ($rule.DisplayName -ne 'Clash of Rust WSL managed endpoint' -or
            [string]$rule.VMCreatorId -ne $script:Creator -or [string]$rule.Action -ne 'Allow' -or
            [string]$rule.Direction -ne 'Outbound' -or [string]$rule.Protocol -notin @('UDP','17') -or
            (@($rule.RemoteAddresses) -join ',') -notin @([string]$State.endpoint_ipv4, ([string]$State.endpoint_ipv4 + '/32')) -or
            (@($rule.RemotePorts) -join ',') -ne [string]$State.endpoint_port) { throw 'Owned rule was changed externally.' }
        Set-NetFirewallHyperVRule -PolicyStore PersistentStore -Name ($script:Prefix + 'Endpoint') `
            -Enabled $(if ($Enabled) { 'True' } else { 'False' }) -ErrorAction Stop
    } else {
        New-NetFirewallHyperVRule -PolicyStore PersistentStore -Name ($script:Prefix + 'Endpoint') `
            -DisplayName 'Clash of Rust WSL managed endpoint' -Direction Outbound `
            -VMCreatorId $script:Creator -Protocol UDP -RemoteAddresses ([string]$State.endpoint_ipv4) `
            -RemotePorts ([string]$State.endpoint_port) -Action Allow `
            -Enabled $(if ($Enabled) { 'True' } else { 'False' }) -ErrorAction Stop | Out-Null
    }
    $State.endpoint_enabled = $Enabled
    $State.endpoint_rule_written = $true
    Save-WslEgressJson $State (Join-Path $script:StateRoot 'state.json')
    Assert-WslEgressBaseline @(Get-WslEgressBaseline)
    if (-not (Get-WslEgressGuardAudit $State).valid) { throw 'Endpoint effective policy verification failed.' }
}

function Start-WslEgressLifetime([object]$State, [object]$Profile) {
    # systemd/kernel tunnel objects do not themselves keep a WSL session alive.
    # A fixed, unprivileged sleep command supplies liveness only. Its death is
    # never permission to bypass either persistent firewall boundary.
    if ([string]$State.manager_distro -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$' -or
        [int]$Profile.uid -lt 1000 -or [int]$Profile.gid -lt 1000) { throw 'Invalid lifetime identity.' }
    $executable = Join-Path $env:SystemRoot 'System32\wsl.exe'
    $arguments = @('--distribution',[string]$State.manager_distro,'--user','root','--exec',
        '/usr/bin/setpriv','--no-new-privs','--reuid',[string][int]$Profile.uid,
        '--regid',[string][int]$Profile.gid,'--clear-groups','--inh-caps=-all',
        '--ambient-caps=-all','--bounding-set=-all','/usr/bin/sleep','infinity') -join ' '
    if ($State.Contains('lifetime') -and $State.lifetime) {
        $old = Get-Process -Id ([int]$State.lifetime.pid) -ErrorAction SilentlyContinue
        if ($old) {
            try {
                if (-not $old.HasExited -and
                    $old.StartTime.ToUniversalTime().Ticks -eq [long]$State.lifetime.started_ticks -and
                    [string]$old.Path -eq $executable -and
                    [string]$State.lifetime.arguments -eq $arguments) { return }
            } finally { $old.Dispose() }
        }
    }
    # Give the native WSL client its own hidden console. Redirecting it to
    # pipes owned by this short-lived broker would close its stdio on exit.
    $process = Start-Process -FilePath $executable -ArgumentList $arguments `
        -WorkingDirectory (Join-Path $env:SystemRoot 'System32') -WindowStyle Hidden -PassThru
    try {
        if ($process.WaitForExit(300)) { throw 'WSL lifetime command exited; transport remains closed.' }
        $State['lifetime'] = [ordered]@{
            pid = $process.Id; started_ticks = $process.StartTime.ToUniversalTime().Ticks
            arguments = $arguments; generation = [long]$State.generation
        }
        Save-WslEgressJson $State (Join-Path $script:StateRoot 'state.json')
    } finally {
        # Disposing the local handle does not kill the independent WSL session.
        $process.Dispose()
    }
}

function Invoke-WslEgressOperation {
    [CmdletBinding()]
    param([string]$Action = 'Doctor', [string]$Backend = 'WireGuard', [string]$Distro = 'COR-Egress',
        [string]$ProfilePath, [string]$EndpointIPv4, [int]$EndpointPort = 51820,
        [switch]$Execute, [switch]$RestoreOriginalNetwork)
    $result = New-WslEgressResponse -Action $Action -Backend $Backend
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
        $result.outcome = 'blocked'; $result.message = 'This host is not Windows.'; return $result
    }
    $sid = Assert-WslEgressUser
    $distributions = @(Get-WslEgressDistributions)
    $registeredOnly = $distributions
    $result.distributions = $distributions
    $state = Read-WslEgressState
    if ($state) {
        if ($state.owner_sid -ne $sid) { throw 'State belongs to a different Windows user.' }
        $result.generation = $state.generation; $result.backend = $state.backend; $result.phase = $state.phase
    }
    $result.operation_id = [guid]::NewGuid().ToString()
    $result.distributions = @($registeredOnly | ForEach-Object {
        [ordered]@{ name = [string]$_.name; state = 'unknown'; coverage = 'not_enrolled' }
    })
    if ($Action -in @('Doctor','Status')) {
        $result.outcome = 'succeeded'
        $result.checks += [ordered]@{ id = 'administrator'; result = $(if (Test-WslEgressAdministrator) { 'PASS' } else { 'NOT_RUN' }); detail = 'Network writes require an administrator.' }
        $result.checks += [ordered]@{ id = 'wsl_runtime'; result = 'NOT_RUN'; detail = 'Registry names only; running state and runtime acceptance are not inferred.' }
        if ($state) {
            $audit = Get-WslEgressGuardAudit $state
            $result.checks += [ordered]@{ id = 'host_policy'; result = $(if ($audit.valid) { 'PASS' } else { 'FAIL' }); detail = ($audit.issues -join ',') }
            # Configuration is not packet-level proof. Never infer verified.
            $result.message = 'Installed policy inspected; end-to-end and cold-boot acceptance are separate.'
        } else { $result.message = 'Not installed; no protection is claimed.' }
        return $result
    }
    if ($Action -eq 'Test') {
        $result.outcome = 'not_run'; $result.phase = 'acceptance_pending'
        $result.message = 'Runtime acceptance requires controlled receiver and reboot evidence; no business target is contacted.'
        $audit = if ($state -and $state.phase -ne 'restored') { Get-WslEgressGuardAudit $state } else { $null }
        $result.checks += [ordered]@{ id = 'host_policy'; result = $(if (-not $audit) { 'NOT_RUN' } elseif ($audit.valid) { 'PASS' } else { 'FAIL' }); detail = 'Static effective policy readback only.' }
        foreach ($check in @('german_exit','dns_path','ipv6_denial','docker_runtime','cold_boot','network_switch','sleep_resume')) {
            $result.checks += [ordered]@{ id = $check; result = 'NOT_RUN'; detail = 'No executed production acceptance evidence supplied.' }
        }
        return $result
    }
    if (-not $Execute) {
        $result.phase = 'preflight'; $result.message = 'Preview only. No files, network settings, or WSL runtime changed.'
        $result.checks += [ordered]@{ id = 'strict_tailscale_boot'; result = 'BLOCKED'; detail = 'No supported synchronous host/Linux guard lifetime binding has been verified.' }
        return $result
    }
    if (-not (Test-WslEgressAdministrator)) {
        $result.outcome = 'blocked'; $result.message = 'Run the fixed installed broker as administrator; no automatic elevation performed.'; return $result
    }
    if ($Action -in @('Install','Start') -and (($state -and $state.backend -eq 'Tailscale') -or (-not $state -and $Backend -eq 'Tailscale'))) {
        $result.outcome = 'blocked'; $result.phase = 'blocked'
        $result.message = 'Tailscale strict all-WSL cold-boot protection is not verified. Use the separately configured WireGuard backend.'
        return $result
    }
    $mutex = New-Object Threading.Mutex($false, 'Global\ClashOfRust.WslEgress.Operation')
    if (-not $mutex.WaitOne(0)) { $mutex.Dispose(); throw 'Another operation is active.' }
    $completionWritten = $false
    $stage = 'baseline'
    try {
        Assert-WslEgressBaseline @(Get-WslEgressBaseline)
        $stage = 'running_distribution_inventory'
        $runningNames = @(Get-WslEgressRunningDistributions)
        foreach ($item in $distributions) {
            $item.state = if ($runningNames -contains $item.name) { 'running' } else { 'stopped' }
        }
        if ($Action -eq 'Install') {
            if ($state) { throw 'Already installed. Use the existing journal; do not overwrite its snapshot.' }
            $stage = 'private_profile_validation'
            $profile = Read-WslEgressProfile $ProfilePath
            if ($profile.backend -ne 'WireGuard') { throw 'Only the strict fixed-endpoint profile can be installed.' }
            if (@($distributions | Where-Object { $_.name -eq $profile.manager_distro }).Count -ne 1) { throw 'Prepare the declared manager distribution before installation.' }
            $stage = 'stopped_distribution_gate'
            if ($runningNames.Count -ne 0) { throw 'Save work and stop all WSL distributions manually before the initial cutover.' }
            $stage = 'docker_inventory'
            $dockerPaths = @(Get-WslEgressDockerPaths)
            $dockerRegistered = @($distributions | Where-Object { $_.name -match '^docker-desktop(?:-data)?$' }).Count -gt 0
            if (@($distributions | Where-Object { $_.name -match '^docker-desktop' }).Count -gt 0 -and $dockerPaths.Count -eq 0) {
                throw 'Docker Desktop is registered but its host backend identity is unknown; no protection is claimed.'
            }
            if (-not $dockerRegistered) { $dockerPaths = @() }
            $stage = 'state_directory'
            Protect-WslEgressDirectory $script:StateRoot
            $state = [ordered]@{ schema_version = 1; owner = 'ClashOfRust.WslEgress'; owner_sid = $sid
                generation = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds(); backend = 'WireGuard'; phase = 'preparing'
                manager_distro = [string]$profile.manager_distro; endpoint_ipv4 = [string]$profile.endpoint_ipv4
                endpoint_port = [int]$profile.endpoint_port; endpoint_enabled = $false
                linux_profile_path = (New-WslEgressUnixPath ([string]$ProfilePath))
                before_persistent_settings = (Get-WslEgressPersistentSettings)
                before_effective_settings = (Get-WslEgressSettings)
                after_settings = $null
                created_at = (Get-Date -Format o); distributions = @($distributions.name)
                docker_backends = $dockerPaths; wslconfig = $null; distro_configs = @()
                settings_written = $false; endpoint_rule_written = $false
                operation_id = [string]$result.operation_id
            }
            Save-WslEgressJson $state (Join-Path $script:StateRoot 'state.json')
            $stage = 'hyperv_guard'
            Set-WslEgressPersistentGuard $state
            $stage = 'docker_guard'
            Set-WslEgressDockerGuard $state
            $stage = 'guardian_service'
            Install-WslEgressGuardian
            $stage = 'windows_config'
            Install-WslEgressConfig $state
            $stage = 'linux_payload'
            Install-WslEgressPayload $state ([string]$state.manager_distro)
            # Only start Linux after the host is already closed. Install receives
            # secrets through stdin, never argv, stdout, or an exported report.
            $linuxProfile = [ordered]@{}
            foreach ($property in $profile.PSObject.Properties) { $linuxProfile[$property.Name] = $property.Value }
            $linuxProfile['manager_distro'] = [string]$state.manager_distro
            $stage = 'linux_install'
            $linux = Invoke-WslEgressLinux $state 'install' ($linuxProfile | ConvertTo-Json -Depth 12 -Compress) `
                -AdditionalArguments @('--acknowledge-shared-guard')
            if ($linux.outcome -ne 'succeeded') { throw 'Linux install did not complete; the host guard remains closed.' }
            $stage = 'distro_configuration'
            Invoke-WslEgressDistroConfiguration $state $profile $distributions
            $state.phase = 'installed_closed'
        } elseif (-not $state) { throw 'Not installed.' }
        elseif ($Action -eq 'Start') {
            $stage = 'host_guard_audit'
            if (-not (Get-WslEgressGuardAudit $state).valid) { throw 'Host guard is not effective.' }
            $profile = Read-WslEgressProfile $ProfilePath
            Assert-WslEgressProfileForState $profile $state
            $stage = 'wsl_lifetime'
            Start-WslEgressLifetime $state $profile
            $stage = 'scope_audit'
            $attestation = Get-WslEgressScopeAttestation $state
            if (-not $attestation.effective_audit_passed) {
                throw 'Current WSL distro, Hyper-V, DNS-tunneling or Docker audit is incomplete; endpoint remains closed.'
            }
            $stage = 'linux_start'
            $linux = Invoke-WslEgressLinux $state 'start' ($attestation | ConvertTo-Json -Depth 8 -Compress)
            if ($linux.outcome -ne 'succeeded') { throw 'Linux start failed; endpoint remains closed.' }
            try {
                $stage = 'endpoint_enable'
                Set-WslEgressEndpoint $state $true
            } catch {
                # Enabling and its mandatory readbacks are one transaction. If
                # any post-enable check fails, restore the closed endpoint and
                # verify that state before surfacing the original failure.
                try {
                    Set-WslEgressEndpoint $state $false
                    if (-not (Get-WslEgressGuardAudit $state).valid) {
                        throw 'Endpoint compensation did not produce a verified closed state.'
                    }
                } catch {
                    throw 'Start failed after opening the endpoint, and automatic close verification also failed.'
                }
                throw
            }
            $state.phase = 'connected_unverified'
        } elseif ($Action -eq 'Stop') {
            Set-WslEgressEndpoint $state $false
            $state.phase = 'installed_closed'
            Save-WslEgressJson $state (Join-Path $script:StateRoot 'state.json')
            if ($runningNames -contains $state.manager_distro) {
                $null = Invoke-WslEgressLinux $state 'stop'
            }
        } elseif ($Action -in @('Rollback','Uninstall')) {
            Set-WslEgressEndpoint $state $false
            if (-not $RestoreOriginalNetwork) {
                $state.phase = 'installed_closed'
                Save-WslEgressJson $state (Join-Path $script:StateRoot 'state.json')
                $result.outcome = 'blocked'; $result.phase = 'installed_closed'
                $result.message = 'Transport closed. Restoring original networking requires -RestoreOriginalNetwork and no running WSL distribution.'
                return $result
            }
            if ($runningNames.Count -gt 0) { throw 'All WSL processes must be stopped manually before restoring host access.' }
            $current = Get-WslEgressSettings
            if (($current | ConvertTo-Json -Compress) -ne ($state.after_settings | ConvertTo-Json -Compress)) { throw 'Host settings changed since installation; automatic restore refused.' }
            $before = if ($state.PSObject.Properties['before_persistent_settings']) {
                $state.before_persistent_settings
            } else {
                $state.before_settings
            }
            # Close the outer transport before asking the manager to remove its
            # owned namespaces and guard. Then restore every per-distro file while
            # the host default deny is still active.
            if ($null -ne $state.distro_configs -and @($state.distro_configs).Count -gt 0) {
                foreach ($name in @($state.distro_configs)) {
                    $restored = Invoke-WslEgressLinux $state 'restore-config' -Distribution $name
                    if ($restored.outcome -ne 'succeeded') { throw "Distro restore failed: $name" }
                    Stop-WslEgressDistribution $name
                }
            }
            $linuxRollback = Invoke-WslEgressLinux $state 'rollback' -AdditionalArguments @('--release-guard')
            if ($linuxRollback.outcome -ne 'succeeded') { throw 'Linux guard rollback did not complete.' }
            Stop-WslEgressDistribution ([string]$state.manager_distro)
            Restore-WslEgressConfig $state
            Set-WslEgressDockerGuard $state $true
            if ($state.settings_written) {
                Set-NetFirewallHyperVVMSetting -PolicyStore PersistentStore -Name $script:Creator `
                    -Enabled $before.Enabled -DefaultInboundAction $before.DefaultInboundAction `
                    -DefaultOutboundAction $before.DefaultOutboundAction -AllowHostPolicyMerge $before.AllowHostPolicyMerge `
                    -LoopbackEnabled $before.LoopbackEnabled -ErrorAction Stop
            }
            Assert-WslEgressBaseline @(Get-WslEgressBaseline)
            if ($state.endpoint_rule_written -or
                @(Get-NetFirewallHyperVRule -PolicyStore PersistentStore -Name ($script:Prefix + 'Endpoint') -ErrorAction SilentlyContinue).Count -gt 0) {
                # Verify ownership against the journal before removal.
                $owned = @(Get-NetFirewallHyperVRule -PolicyStore PersistentStore -Name ($script:Prefix + 'Endpoint') -ErrorAction SilentlyContinue)
                if ($owned.Count -gt 1 -or ($owned.Count -eq 1 -and
                    ($owned[0].DisplayName -ne 'Clash of Rust WSL managed endpoint' -or
                     [string]$owned[0].VMCreatorId -ne $script:Creator))) {
                    throw 'Owned endpoint rule changed; preserving it for manual review.'
                }
                if ($owned.Count -eq 1) {
                    Remove-NetFirewallHyperVRule -PolicyStore PersistentStore -Name ($script:Prefix + 'Endpoint') -ErrorAction Stop
                }
            }
            Assert-WslEgressBaseline @(Get-WslEgressBaseline)
            $state.phase = 'restored'
        }
        Save-WslEgressJson $state (Join-Path $script:StateRoot 'state.json')
        Assert-WslEgressBaseline @(Get-WslEgressBaseline)
        $result.outcome = 'succeeded'; $result.phase = $state.phase
        $result.operation_id = [string]$state.operation_id
        $result.generation = $state.generation; $result.backend = $state.backend
        $result.distributions = @($state.distributions | ForEach-Object {
            [ordered]@{
                name = $_
                state = 'unknown'
                coverage = $(if ($_ -match '^docker-desktop') { 'blocked' } else { 'enrolled' })
            }
        })
        $result.message = 'Operation completed. Packet-level, exit-location, and reboot verification remain separate.'
        $completion = [ordered]@{
            schema_version = 1; operation_id = [string]$result.operation_id
            generation = [long]$state.generation; action = $Action
            outcome = 'succeeded'; phase = [string]$state.phase; backend = [string]$state.backend
            completed_at = (Get-Date -Format o)
        }
        Save-WslEgressJson $completion (Join-Path $env:LOCALAPPDATA ('ClashOfRust\WslEgress\results\' + [string]$result.operation_id + '.json'))
        $completionWritten = $true
        return $result
    } catch {
        if ($state -and $Action -eq 'Start' -and $stage -ne 'baseline') {
            try { Set-WslEgressEndpoint $state $false } catch { }
        }
        $closure = Get-WslEgressClosureAudit $state
        $failurePhase = if ($closure.confirmed) { 'failed_closed_requested' } else { 'closure_unverified' }
        if ($state) {
            $state.phase = $failurePhase
            Save-WslEgressJson $state (Join-Path $script:StateRoot 'state.json')
            $result.generation = [long]$state.generation
            $result.backend = [string]$state.backend
        }
        $result.outcome = 'failed'
        $result.phase = $failurePhase
        $result.message = if ($closure.confirmed) {
            'Operation failed. Effective host policy confirms the endpoint is disabled; Linux acceptance remains separate.'
        } else {
            'Operation failed. Endpoint closure could not be confirmed; inspect the protected journal before further use.'
        }
        $result.checks += [ordered]@{
            id = 'endpoint_closure'; result = $(if ($closure.confirmed) { 'PASS' } else { 'FAIL' })
            detail = 'Effective default-deny and disabled endpoint readback; no packet-level inference.'
        }
        $result.checks += [ordered]@{ id = 'failure_stage'; result = 'FAIL'; detail = $stage }
        $result.checks += [ordered]@{
            id = 'exception_class'; result = 'FAIL'
            detail = $_.Exception.GetType().Name + ':line=' + [string]$_.InvocationInfo.ScriptLineNumber
        }
        if ($state -and -not $completionWritten) {
            $completion = [ordered]@{
                schema_version = 1; operation_id = [string]$result.operation_id
                generation = [long]$state.generation; action = $Action
                outcome = 'failed'; phase = $failurePhase; backend = [string]$state.backend
                completed_at = (Get-Date -Format o)
            }
            try {
                Save-WslEgressJson $completion (Join-Path $env:LOCALAPPDATA ('ClashOfRust\WslEgress\results\' + [string]$result.operation_id + '.json'))
            } catch { }
        }
        return $result
    } finally { $mutex.ReleaseMutex(); $mutex.Dispose() }
}

Export-ModuleMember -Function Invoke-WslEgressOperation,New-WslEgressResponse,Invoke-WslEgressGuardianAudit,Test-WslEgressGuardianServicePath
