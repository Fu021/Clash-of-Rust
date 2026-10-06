param([string]$Proxy = '', [switch]$SkipPrepare, [string]$Version = '')
$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$taskManifest = Get-Content -LiteralPath (Join-Path $taskRoot 'Cargo.toml') -Raw -Encoding UTF8
$taskPackageVersion = [regex]::Match($taskManifest, '(?m)^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"').Groups[1].Value
$taskAppVersion = if ($Version) { $Version } else { $taskPackageVersion }
if ($taskAppVersion -notmatch '^[0-9]+\.[0-9]+\.[0-9]+$' -or @($taskAppVersion.Split('.') | Where-Object { [long]$_ -gt 65535 }).Count) {
    throw 'Version must contain three numeric components between 0 and 65535'
}
if (-not $SkipPrepare) { & (Join-Path $PSScriptRoot 'prepare-resources.ps1') -Proxy $Proxy }
foreach ($taskResource in @('default.yaml', 'settings-defaults.json', 'THIRD-PARTY-NOTICES.txt')) {
    Copy-Item -LiteralPath (Join-Path $taskRoot "resources\$taskResource") -Destination (Join-Path $taskRoot 'bundle\resources') -Force
}
Copy-Item -LiteralPath (Join-Path $taskRoot 'LICENSE') -Destination (Join-Path $taskRoot 'bundle\LICENSE') -Force
$taskIpResources = Join-Path $taskRoot 'bundle\resources\ip-check'
New-Item -ItemType Directory -Path $taskIpResources -Force | Out-Null
Copy-Item -Path (Join-Path $taskRoot 'vendor\region-restriction-check\*') -Destination $taskIpResources -Recurse -Force
if (-not (Test-Path -LiteralPath (Join-Path $taskIpResources 'runtime\manifest.json'))) {
    $taskPython = Get-Command py.exe -ErrorAction SilentlyContinue
    if ($taskPython) { & $taskPython.Source -3 (Join-Path $taskRoot 'scripts\prepare-ip-check.py') --proxy $Proxy }
    else { & python.exe (Join-Path $taskRoot 'scripts\prepare-ip-check.py') --proxy $Proxy }
    if ($LASTEXITCODE -ne 0) { throw 'IP detector runtime preparation failed' }
}
$compiler = Join-Path $taskRoot 'tools\nsis\nsis-3.13\makensis.exe'
if (-not (Test-Path -LiteralPath $compiler)) {
    $archive = Join-Path $taskRoot 'tools\nsis-3.13.zip'
    New-Item -ItemType Directory -Path (Join-Path $taskRoot 'tools') -Force | Out-Null
    $downloadArgs = @('--fail','--location','--retry','2','--connect-timeout','15','--max-time','120','--output',$archive)
    if ($Proxy) { $downloadArgs += @('--proxy',$Proxy) }
    $downloadArgs += 'https://downloads.sourceforge.net/project/nsis/NSIS%203/3.13/nsis-3.13.zip'
    & curl.exe @downloadArgs
    if ($LASTEXITCODE -ne 0) { throw 'Portable NSIS download failed' }
    Expand-Archive -LiteralPath $archive -DestinationPath (Join-Path $taskRoot 'tools\nsis') -Force
}
Push-Location $taskRoot
$taskPreviousVersion = [Environment]::GetEnvironmentVariable('CLASH_OF_RUST_BUILD_VERSION', 'Process')
try {
    $env:CLASH_OF_RUST_BUILD_VERSION = $taskAppVersion
    & cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
    $taskBinary = Join-Path $taskRoot 'target\release\clash-of-rust.exe'
    if ([Diagnostics.FileVersionInfo]::GetVersionInfo($taskBinary).FileVersion -ne $taskAppVersion) {
        throw 'Application version does not match the requested installer version'
    }
    Copy-Item -LiteralPath (Join-Path $taskRoot 'target\release\clash-of-rust.exe') -Destination (Join-Path $taskRoot 'bundle\clash-of-rust.exe') -Force
    New-Item -ItemType Directory -Path (Join-Path $taskRoot 'dist') -Force | Out-Null
    & $compiler /V2 /INPUTCHARSET UTF8 "/DAPP_VERSION=$taskAppVersion" "/DPAYLOAD=$taskRoot\bundle" "/DOUTPUT=$taskRoot\dist\Clash-of-Rust-$taskAppVersion-windows-x64-setup.exe" (Join-Path $taskRoot 'installer\clash-of-rust.nsi')
    if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed' }
    $setup = Join-Path $taskRoot "dist\Clash-of-Rust-$taskAppVersion-windows-x64-setup.exe"
    if ([Diagnostics.FileVersionInfo]::GetVersionInfo($setup).FileVersion -ne $taskAppVersion) {
        throw 'Installer version does not match the requested version'
    }
    (Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash.ToLowerInvariant() | Set-Content -Encoding ASCII -LiteralPath "$setup.sha256"
    Write-Output "Installer ready: $setup"
} finally {
    [Environment]::SetEnvironmentVariable('CLASH_OF_RUST_BUILD_VERSION', $taskPreviousVersion, 'Process')
    Pop-Location
}
