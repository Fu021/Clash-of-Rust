param(
    [string]$Version = 'v1.19.32',
    [string]$Proxy = ''
)
$ErrorActionPreference = 'Stop'
$taskRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$taskBin = Join-Path $taskRoot 'bin'
New-Item -ItemType Directory -Path $taskBin -Force | Out-Null
$taskCore = Join-Path $taskBin 'mihomo.exe'
if (Test-Path -LiteralPath $taskCore) {
    Write-Output "Core already exists: $taskCore"
    exit 0
}
$apiParams = @{ Uri = "https://api.github.com/repos/MetaCubeX/mihomo/releases/tags/$Version"; TimeoutSec = 30 }
if ($Proxy) { $apiParams.Proxy = $Proxy }
$release = Invoke-RestMethod @apiParams
$asset = $release.assets | Where-Object { $_.name -eq "mihomo-windows-amd64-compatible-$Version.zip" } | Select-Object -First 1
if (-not $asset) { throw "Compatible Windows x64 asset missing in $Version" }
$taskArchive = Join-Path $taskBin $asset.name
$curlArgs = @('--fail', '--location', '--retry', '2', '--connect-timeout', '15', '--max-time', '180', '--output', $taskArchive)
if ($Proxy) { $curlArgs += @('--proxy', $Proxy) }
$curlArgs += $asset.browser_download_url
& curl.exe @curlArgs
if ($LASTEXITCODE -ne 0) { throw 'Core download failed' }
if (-not $asset.digest -or -not $asset.digest.StartsWith('sha256:')) { throw 'Official asset SHA256 digest missing; refusing unverified extraction' }
$actual = (Get-FileHash -LiteralPath $taskArchive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne $asset.digest.Substring(7)) { throw 'Core archive SHA256 mismatch' }
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::OpenRead($taskArchive)
try {
    $entry = $archive.Entries | Where-Object { $_.Name -like 'mihomo*.exe' } | Select-Object -First 1
    if (-not $entry) { throw 'Core executable missing in archive' }
    [System.IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $taskCore, $false)
} finally { $archive.Dispose() }
Write-Output "Verified mihomo $Version downloaded: $taskCore"
