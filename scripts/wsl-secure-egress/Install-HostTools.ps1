#Requires -Version 5.1
<#
.SYNOPSIS
Install the locally built guardian and reviewed WSL resources in Program Files.
.DESCRIPTION
No second desktop client is installed. This helper package exists solely for
the fixed SYSTEM guardian and the administrator broker. Preview by default.
The source guardian must be freshly built from this worktree before use.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$GuardianExe,
    [switch]$UpdatePreparedPackage,
    [switch]$UpdateGuardedPackage,
    [string]$BackupDirectory,
    [switch]$Execute
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$destination = Join-Path $env:ProgramFiles 'Clash of Rust WSL'
$source = $PSScriptRoot
$top = @('DockerEgressGuard.cs','Guardian-Audit.ps1','Uninstall-Check.ps1',
    'Manage-WslSecureEgress.ps1','WslSecureEgress.psm1')
$linux = @('CONTRACT.md','cor-egress-manager','install-payload.py','manager','manager.py','secure-egress')
$server = @('cor-wsl-egress.service','manage.py')
$files = [ordered]@{ 'clash-wsl-guardian.exe' = [IO.Path]::GetFullPath($GuardianExe) }
foreach ($name in $top) { $files["resources\wsl-secure-egress\$name"] = Join-Path $source $name }
foreach ($name in $linux) { $files["resources\wsl-secure-egress\linux\$name"] = Join-Path $source "linux\$name" }
foreach ($name in $server) { $files["resources\wsl-secure-egress\server\$name"] = Join-Path $source "server\$name" }
$hashes = [ordered]@{}
foreach ($entry in $files.GetEnumerator()) {
    if (-not (Test-Path -LiteralPath $entry.Value -PathType Leaf)) { throw 'A required build/resource file is missing.' }
    if ((Get-Item -LiteralPath $entry.Value).Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw 'Redirected package source refused.'
    }
    $hashes[$entry.Key] = (Get-FileHash -LiteralPath $entry.Value -Algorithm SHA256).Hash
}
if (-not $Execute) {
    [ordered]@{outcome='not_run';files=$hashes.Count;message='Build the guardian from the reviewed source before execution.'} |
        ConvertTo-Json -Compress
    return
}
$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Administrator token required.' }
$existing = Test-Path -LiteralPath $destination
$restartService = $false
if ($existing) {
    if ((Get-Item -LiteralPath $destination).Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw 'Redirected installed package refused.'
    }
    if (-not $UpdatePreparedPackage -and -not $UpdateGuardedPackage) {
        throw 'An explicit reviewed package-update operation is required.'
    }
    $hasState = Test-Path -LiteralPath (Join-Path $env:ProgramData 'ClashOfRust\WslEgress\state.json')
    $service = Get-CimInstance Win32_Service -Filter "Name='ClashOfRust.WslEgress'"
    if (-not $UpdateGuardedPackage -and ($hasState -or $service)) {
        throw 'Prepared-package update cannot change a guarded installation.'
    }
    $receiptPath = Join-Path $destination 'installed-files.json'
    $receipt = Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json
    if (@($receipt.PSObject.Properties).Count -ne $files.Count) { throw 'Package receipt differs.' }
    foreach ($name in $files.Keys) {
        $property = $receipt.PSObject.Properties[$name]
        if (-not $property -or
            (Get-FileHash -LiteralPath (Join-Path $destination $name) -Algorithm SHA256).Hash -ne $property.Value) {
            throw 'Prepared package was modified; preserving it.'
        }
    }
    if ($UpdateGuardedPackage) {
        if (-not $hasState -or -not $service -or -not $BackupDirectory) { throw 'Guarded update requires state, service, and a new code-only backup directory.' }
        $expectedService = '"' + (Join-Path $destination 'clash-wsl-guardian.exe') + '" --service'
        if ($service.PathName -ne $expectedService -or $service.StartName -notin @('LocalSystem','NT AUTHORITY\SYSTEM')) {
            throw 'Installed audit service identity differs.'
        }
        $installed = Import-Module (Join-Path $destination 'resources\wsl-secure-egress\WslSecureEgress.psm1') -Force -PassThru
        & $installed {
            $state = Read-WslEgressState
            if (-not $state -or $state.endpoint_enabled -or -not (Get-WslEgressGuardAudit $state).valid) {
                throw 'Guarded update requires a verified closed endpoint and effective default deny.'
            }
            if ($state.owner_sid -ne (Assert-WslEgressUser)) { throw 'Different installation owner.' }
            Assert-WslEgressBaseline @(Get-WslEgressBaseline)
        }
        if (-not [IO.Path]::IsPathRooted($BackupDirectory) -or (Test-Path -LiteralPath $BackupDirectory)) {
            throw 'Use a new absolute backup directory.'
        }
        New-Item -ItemType Directory -Path $BackupDirectory | Out-Null
        foreach ($name in $files.Keys) {
            $target = Join-Path $BackupDirectory $name
            New-Item -ItemType Directory -Path ([IO.Path]::GetDirectoryName($target)) -Force | Out-Null
            Copy-Item -LiteralPath (Join-Path $destination $name) -Destination $target
            if ((Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash -ne $receipt.PSObject.Properties[$name].Value) {
                throw 'Code backup digest mismatch; no helper files changed.'
            }
        }
        Copy-Item -LiteralPath $receiptPath -Destination (Join-Path $BackupDirectory 'installed-files.json')
        $restartService = $service.State -eq 'Running'
        Stop-Service -Name 'ClashOfRust.WslEgress' -ErrorAction Stop
        if ($service.ProcessId -gt 0) {
            $oldProcess = Get-Process -Id $service.ProcessId -ErrorAction SilentlyContinue
            if ($oldProcess -and -not $oldProcess.WaitForExit(5000)) { throw 'Audit service process did not exit; closed network policy retained.' }
        }
    }
}
$ancestor = [IO.Path]::GetDirectoryName($destination)
while ($ancestor) {
    if ((Get-Item -LiteralPath $ancestor).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Redirected install parent refused.' }
    $ancestor = [IO.Path]::GetDirectoryName($ancestor)
}
New-Item -ItemType Directory -Path $destination -Force | Out-Null
# Program Files may have inherited unusual write permissions. Set a fixed ACL:
# administrators/SYSTEM write, ordinary users read/execute only.
$acl = New-Object Security.AccessControl.DirectorySecurity
$acl.SetAccessRuleProtection($true, $false)
foreach ($item in @(@('S-1-5-18','FullControl'),@('S-1-5-32-544','FullControl'),@('S-1-5-32-545','ReadAndExecute'))) {
    $rule = New-Object Security.AccessControl.FileSystemAccessRule(
        (New-Object Security.Principal.SecurityIdentifier($item[0])), $item[1],
        'ContainerInherit,ObjectInherit', 'None', 'Allow')
    $acl.AddAccessRule($rule)
}
$acl.SetOwner((New-Object Security.Principal.SecurityIdentifier('S-1-5-32-544')))
Set-Acl -LiteralPath $destination -AclObject $acl
foreach ($entry in $files.GetEnumerator()) {
    if ((Get-FileHash -LiteralPath $entry.Value -Algorithm SHA256).Hash -ne $hashes[$entry.Key]) {
        throw 'Source changed during installation; partial helper package retained.'
    }
    $target = Join-Path $destination $entry.Key
    New-Item -ItemType Directory -Path ([IO.Path]::GetDirectoryName($target)) -Force | Out-Null
    Copy-Item -LiteralPath $entry.Value -Destination $target -Force
    if ((Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash -ne $hashes[$entry.Key]) { throw 'Installed resource digest mismatch.' }
}
[IO.File]::WriteAllText((Join-Path $destination 'installed-files.json'),
    ($hashes | ConvertTo-Json), (New-Object Text.UTF8Encoding($false)))
if ($restartService) { Start-Service -Name 'ClashOfRust.WslEgress' -ErrorAction Stop }
[ordered]@{outcome='succeeded';files=$hashes.Count;service_started=$restartService;network_changed=$false} |
    ConvertTo-Json -Compress
