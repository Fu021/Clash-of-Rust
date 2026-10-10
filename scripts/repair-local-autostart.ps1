param(
    [Parameter(Mandatory=$true)][string]$ExpectedSid,
    [Parameter(Mandatory=$true)][string]$ExpectedExecutable,
    [Parameter(Mandatory=$true)][string]$Report
)
$ErrorActionPreference = 'Stop'
try {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    if ($identity.User.Value -ne $ExpectedSid) { throw 'Repair must run as the original Windows account' }
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Repair requires elevation'
    }
    $marker = Get-ItemProperty 'HKCU:\Software\ClashOfRust\Autostart'
    if ($marker.Executable -ine $ExpectedExecutable) { throw 'Installed startup marker does not match' }
    $service = New-Object -ComObject 'Schedule.Service'
    $service.Connect()
    $task = $service.GetFolder('\').GetTask("ClashOfRust-$ExpectedSid")
    if ($task.Definition.Actions.Count -ne 1 -or
        $task.Definition.Actions.Item(1).Path -ine $ExpectedExecutable) {
        throw 'Startup task belongs to a different installation'
    }
    $before = $task.Enabled
    $oldSecurity = $task.GetSecurityDescriptor(4)
    $task.SetSecurityDescriptor("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;$ExpectedSid)", 0)
    if ($task.Enabled -ne $before) { throw 'Task enabled state changed unexpectedly' }
    @{ success=$true; enabled=$before; old_security=$oldSecurity;
       new_security=$task.GetSecurityDescriptor(4) } | ConvertTo-Json | Set-Content -LiteralPath $Report -Encoding UTF8
    exit 0
} catch {
    @{ success=$false; error=$_.Exception.Message } | ConvertTo-Json | Set-Content -LiteralPath $Report -Encoding UTF8
    exit 1
}
