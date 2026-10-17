#Requires -Version 5.1
param([switch]$ExecuteIsolated)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $ExecuteIsolated -or $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
    throw 'Real WFP tests require an explicitly opted-in GitHub-hosted disposable runner.'
}
$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'CI requires administrative WFP access.' }
Add-Type -Path (Join-Path $PSScriptRoot '../DockerEgressGuard.cs')
[CorDockerEgressGuard]::ValidateLayout()
$directory = Join-Path ([IO.Path]::GetTempPath()) ('cor-wfp-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $directory | Out-Null
$probe = Join-Path $directory 'cor-owned-network-probe.exe'
$source = @'
using System;
using System.Net;
using System.Net.Sockets;
using System.Text;
public static class CorOwnedProbe {
    public static int Main(string[] args) {
        try {
            IPAddress ip=IPAddress.Parse(args[0]); int port=Int32.Parse(args[1]);
            byte[] nonce=Encoding.ASCII.GetBytes(args[3]);
            using(Socket socket=new Socket(ip.AddressFamily,args[2]=="tcp"?SocketType.Stream:SocketType.Dgram,
                args[2]=="tcp"?ProtocolType.Tcp:ProtocolType.Udp)) {
                socket.SendTimeout=1500; socket.ReceiveTimeout=1500;
                socket.Connect(new IPEndPoint(ip,port)); socket.Send(nonce);
                byte[] received=new byte[nonce.Length]; int count=socket.Receive(received);
                if(count!=nonce.Length || Encoding.ASCII.GetString(received)!=args[3]) return 3;
                return 0;
            }
        } catch(SocketException e) { return e.SocketErrorCode==SocketError.AccessDenied?13:4; }
        catch { return 5; }
    }
}
'@
Add-Type -TypeDefinition $source -OutputAssembly $probe -OutputType ConsoleApplication
$cases = @(); $installed = $false
function Invoke-LoopbackProbe([string]$Address,[string]$Protocol,[bool]$Blocked) {
    $ip = [Net.IPAddress]::Parse($Address)
    $socketType = if ($Protocol -eq 'tcp') { [Net.Sockets.SocketType]::Stream } else { [Net.Sockets.SocketType]::Dgram }
    $protocolType = if ($Protocol -eq 'tcp') { [Net.Sockets.ProtocolType]::Tcp } else { [Net.Sockets.ProtocolType]::Udp }
    $listener = New-Object Net.Sockets.Socket($ip.AddressFamily,$socketType,$protocolType)
    try {
        $listener.Bind((New-Object Net.IPEndPoint($ip,0)))
        $port = $listener.LocalEndPoint.Port
        if ($Protocol -eq 'tcp') { $listener.Listen(1) }
        $nonce = [guid]::NewGuid().ToString('N')
        $start = New-Object Diagnostics.ProcessStartInfo
        $start.FileName=$probe; $start.UseShellExecute=$false; $start.CreateNoWindow=$true
        $start.Arguments="$Address $port $Protocol $nonce"
        $process = [Diagnostics.Process]::Start($start)
        $received = $false
        $ready = $listener.Poll(2500000,[Net.Sockets.SelectMode]::SelectRead)
        if ($ready) {
            $connection = if ($Protocol -eq 'tcp') { $listener.Accept() } else { $listener }
            try {
                $connection.ReceiveTimeout=1500
                $bytes=New-Object byte[] 64
                if ($Protocol -eq 'tcp') {
                    $count=$connection.Receive($bytes)
                    if ([Text.Encoding]::ASCII.GetString($bytes,0,$count) -eq $nonce) { $received=$true }
                    $null=$connection.Send($bytes,0,$count,[Net.Sockets.SocketFlags]::None)
                } else {
                    [Net.EndPoint]$remote=New-Object Net.IPEndPoint($ip,0)
                    $count=$connection.ReceiveFrom($bytes,[ref]$remote)
                    if ([Text.Encoding]::ASCII.GetString($bytes,0,$count) -eq $nonce) { $received=$true }
                    $null=$connection.SendTo($bytes,0,$count,[Net.Sockets.SocketFlags]::None,$remote)
                }
            } finally { if ($Protocol -eq 'tcp') { $connection.Dispose() } }
        }
        if (-not $process.WaitForExit(5000)) { $process.Kill(); throw 'Probe timeout; insufficient evidence.' }
        $exitCode=$process.ExitCode; $process.Dispose()
        if ($Blocked) {
            # Winsock may surface a WFP block as a timeout or a fixed socket
            # error instead of AccessDenied. The security fact is the absence
            # of the receiver nonce; the exact error remains evidence.
            if ($received -or $exitCode -eq 0) { throw 'Expected blocked delivery and no receiver nonce.' }
        } elseif (-not $received -or $exitCode -ne 0) { throw 'Receiver positive control failed.' }
        [ordered]@{ address_family = [string]$ip.AddressFamily; protocol=$Protocol
            expected_block=$Blocked; receiver_nonce_seen=$received; probe_exit=$exitCode; result='PASS' }
    } finally { $listener.Dispose() }
}
try {
    foreach ($address in @('127.0.0.1','::1')) { foreach ($protocol in @('tcp','udp')) {
        $cases += Invoke-LoopbackProbe $address $protocol $false
    } }
    [CorDockerEgressGuard]::Apply([string[]]@($probe),$false); $installed=$true
    if (-not [CorDockerEgressGuard]::Verify([string[]]@($probe))) { throw 'WFP readback did not match.' }
    foreach ($address in @('127.0.0.1','::1')) { foreach ($protocol in @('tcp','udp')) {
        $cases += Invoke-LoopbackProbe $address $protocol $true
    } }
    [CorDockerEgressGuard]::Apply([string[]]@($probe),$true); $installed=$false
    foreach ($address in @('127.0.0.1','::1')) { foreach ($protocol in @('tcp','udp')) {
        $cases += Invoke-LoopbackProbe $address $protocol $false
    } }
    [ordered]@{suite='windows-wfp-isolated-probe';result='PASS';cases=$cases
        scope='Dedicated disposable probe only; real Docker/WSL/reboot acceptance NOT_RUN'} | ConvertTo-Json -Depth 6
} finally {
    if ($installed) { [CorDockerEgressGuard]::Apply([string[]]@($probe),$true) }
    # Preserve files for runner diagnostics. No recursive deletion or host reset.
}
