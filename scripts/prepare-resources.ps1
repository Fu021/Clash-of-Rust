param([string]$Proxy = '', [string]$CoreVersion = 'v1.19.32')
$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$taskResources = Join-Path $taskRoot 'bundle\resources'
New-Item -ItemType Directory -Path $taskResources -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $taskRoot 'bin') -Force | Out-Null
function Get-Release([string]$Uri) {
    $params = @{Uri=$Uri; TimeoutSec=30}
    if ($Proxy) { $params.Proxy=$Proxy }
    Invoke-RestMethod @params
}
function Download-Verified($Asset, [string]$Destination) {
    if (-not $Asset.digest -or -not $Asset.digest.StartsWith('sha256:')) { throw "Missing SHA256: $($Asset.name)" }
    $expected = $Asset.digest.Substring(7).ToLowerInvariant()
    if ((Test-Path -LiteralPath $Destination) -and (Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash.ToLowerInvariant() -eq $expected) { return }
    $partial = "$Destination.partial"
    $argsList = @('--fail','--location','--retry','2','--connect-timeout','15','--max-time','180','--output',$partial)
    if ($Proxy) { $argsList += @('--proxy',$Proxy) }
    $argsList += $Asset.browser_download_url
    & curl.exe @argsList
    if ($LASTEXITCODE -ne 0) { throw "Download failed: $($Asset.name)" }
    if ((Get-FileHash -LiteralPath $partial -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) { throw "SHA256 mismatch: $($Asset.name)" }
    Move-Item -LiteralPath $partial -Destination $Destination -Force
}
$coreRelease = Get-Release "https://api.github.com/repos/MetaCubeX/mihomo/releases/tags/$CoreVersion"
$coreAsset = $coreRelease.assets | Where-Object { $_.name -eq "mihomo-windows-amd64-compatible-$CoreVersion.zip" } | Select-Object -First 1
if (-not $coreAsset) { throw 'Official Windows x64 core asset not found' }
$taskArchive = Join-Path $taskRoot "bin\$($coreAsset.name)"
Download-Verified $coreAsset $taskArchive
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::OpenRead($taskArchive)
try {
    $entry = $archive.Entries | Where-Object { $_.Name -like 'mihomo*.exe' } | Select-Object -First 1
    if (-not $entry) { throw 'Core executable missing from archive' }
    [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path $taskResources 'mihomo.exe'), $true)
} finally { $archive.Dispose() }
$geoRelease = Get-Release 'https://api.github.com/repos/MetaCubeX/meta-rules-dat/releases/latest'
$mapping = [ordered]@{'geoip.dat'='GeoIP.dat';'geosite.dat'='GeoSite.dat';'country.mmdb'='Country.mmdb';'GeoLite2-ASN.mmdb'='ASN.mmdb'}
$records = @()
foreach ($remoteName in $mapping.Keys) {
    $asset = $geoRelease.assets | Where-Object { $_.name -eq $remoteName } | Select-Object -First 1
    if (-not $asset) { throw "Geo release missing $remoteName" }
    $name = $mapping[$remoteName]
    Download-Verified $asset (Join-Path $taskResources $name)
    $records += @{name=$name;source=$asset.browser_download_url;sha256=$asset.digest.Substring(7).ToLowerInvariant();size=[long]$asset.size}
}
$manifest = @{version="$($geoRelease.tag_name) | $($geoRelease.published_at)";updated=[DateTimeOffset]::UtcNow.ToUnixTimeSeconds();files=$records}
$encoding = New-Object Text.UTF8Encoding($false)
[IO.File]::WriteAllText((Join-Path $taskResources 'geodata.json'), ($manifest | ConvertTo-Json -Depth 5), $encoding)
Copy-Item -LiteralPath (Join-Path $taskRoot 'resources\default.yaml') -Destination $taskResources -Force
Copy-Item -LiteralPath (Join-Path $taskRoot 'resources\settings-defaults.json') -Destination $taskResources -Force
Copy-Item -LiteralPath (Join-Path $taskRoot 'LICENSE') -Destination (Join-Path $taskRoot 'bundle\LICENSE') -Force
Copy-Item -LiteralPath (Join-Path $taskRoot 'resources\THIRD-PARTY-NOTICES.txt') -Destination $taskResources -Force
$coreRecord = @{version=$CoreVersion;source=$coreAsset.browser_download_url;archive_sha256=$coreAsset.digest.Substring(7);exe_sha256=(Get-FileHash -LiteralPath (Join-Path $taskResources 'mihomo.exe') -Algorithm SHA256).Hash.ToLowerInvariant()}
[IO.File]::WriteAllText((Join-Path $taskResources 'core.json'), ($coreRecord | ConvertTo-Json), $encoding)
Write-Output 'Verified offline core, four Geo databases and default configuration prepared.'
