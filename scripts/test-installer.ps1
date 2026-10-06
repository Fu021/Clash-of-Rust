$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$compiler = Join-Path $taskRoot 'tools\nsis\nsis-3.13\makensis.exe'
$taskTestRoot = Join-Path $taskRoot ('dist\installer-test-' + [Guid]::NewGuid().ToString('N'))
$taskInstall = Join-Path $taskTestRoot 'installed'
$taskSetup = Join-Path $taskTestRoot 'smoke-setup.exe'
New-Item -ItemType Directory -Path $taskTestRoot -Force | Out-Null
$testKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ClashOfRustInstallerSmokeTest'
$legacyKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ClashOfRustLegacySmokeTest'
if (Test-Path -LiteralPath $legacyKey) { throw 'Existing legacy smoke-test entry; refusing to overwrite it' }
if (Test-Path -LiteralPath $testKey) { throw 'An existing installer smoke-test entry is present; refusing to overwrite it' }
& $compiler /V2 /INPUTCHARSET UTF8 /DINSTALLER_TESTING "/DPAYLOAD=$taskRoot\bundle" "/DOUTPUT=$taskSetup" (Join-Path $taskRoot 'installer\clash-of-rust.nsi')
if ($LASTEXITCODE -ne 0) { throw 'Smoke-test installer compilation failed' }
$first = Start-Process -FilePath $taskSetup -ArgumentList '/S',"/D=$taskInstall" -WindowStyle Hidden -Wait -PassThru
if ($first.ExitCode -ne 0) { throw "Fresh installation failed: $($first.ExitCode)" }
foreach ($name in @('GeoIP.dat','GeoSite.dat','Country.mmdb','ASN.mmdb','mihomo.exe','default.yaml','settings-defaults.json','geodata.json')) {
    if (-not (Test-Path -LiteralPath (Join-Path $taskInstall "resources\$name"))) { throw "Installed asset missing: $name" }
}
if (-not (Test-Path -LiteralPath (Join-Path $taskInstall 'clash-of-rust.exe'))) { throw 'Application executable missing' }
$taskRunningMutex=[System.Threading.Mutex]::new($false, 'Local\ClashOfRust.InstallerSmoke')
try {
    $running = Start-Process -FilePath $taskSetup -ArgumentList '/S',"/D=$taskInstall" -WindowStyle Hidden -Wait -PassThru
    if ($running.ExitCode -ne 3) { throw 'Installer did not reject a running application' }
} finally { $taskRunningMutex.Dispose() }
$second = Start-Process -FilePath $taskSetup -ArgumentList '/S',"/D=$taskInstall" -WindowStyle Hidden -Wait -PassThru
if ($second.ExitCode -ne 2) { throw "Existing-install detection failed: expected 2, got $($second.ExitCode)" }
Set-Content -LiteralPath (Join-Path $taskInstall 'user-file.txt') -Value 'Must survive uninstall'
$reinstall = Start-Process -FilePath $taskSetup -ArgumentList '/S','/TESTREINSTALL',"/D=$taskInstall" -WindowStyle Hidden -Wait -PassThru
if ($reinstall.ExitCode -ne 0) { throw "Confirmed uninstall/reinstall failed: $($reinstall.ExitCode)" }
if (-not (Test-Path -LiteralPath (Join-Path $taskInstall 'clash-of-rust.exe'))) { throw 'Reinstallation did not restore the application' }
if (-not (Test-Path -LiteralPath (Join-Path $taskInstall 'user-file.txt'))) { throw 'Reinstallation removed unrelated user data' }
$uninstall = Join-Path $taskInstall 'uninstall.exe'
$removed = Start-Process -FilePath $uninstall -ArgumentList '/S',"_?=$taskInstall" -WindowStyle Hidden -Wait -PassThru
if ($removed.ExitCode -ne 0) { throw "Uninstall failed: $($removed.ExitCode)" }
if (Test-Path -LiteralPath (Join-Path $taskInstall 'clash-of-rust.exe')) { throw 'Application was not uninstalled' }
if (Test-Path -LiteralPath $testKey) { throw 'Uninstall registration was not removed' }
if (-not (Test-Path -LiteralPath (Join-Path $taskInstall 'user-file.txt'))) { throw 'Uninstall removed a file it does not own' }
$legacySetup = Join-Path $taskTestRoot 'legacy-setup.exe'
$legacyInstall = Join-Path $taskTestRoot 'old-user-install'
& $compiler /V2 /INPUTCHARSET UTF8 "/DPAYLOAD=$taskRoot\bundle" "/DOUTPUT=$legacySetup" (Join-Path $taskRoot 'installer\legacy-test.nsi')
if ($LASTEXITCODE -ne 0) { throw 'Legacy fixture compilation failed' }
$legacy = Start-Process -FilePath $legacySetup -ArgumentList '/S',"/D=$legacyInstall" -WindowStyle Hidden -Wait -PassThru
if ($legacy.ExitCode -ne 0) { throw 'Legacy fixture installation failed' }
Set-Content -LiteralPath (Join-Path $legacyInstall 'user-file.txt') -Value 'Must survive migration'
$migrated = Start-Process -FilePath $taskSetup -ArgumentList '/S','/TESTREINSTALL',"/D=$taskInstall" -WindowStyle Hidden -Wait -PassThru
if ($migrated.ExitCode -ne 0) { throw 'Legacy migration failed' }
if (Test-Path -LiteralPath $legacyKey) { throw 'Legacy registration was not removed' }
if (Test-Path -LiteralPath (Join-Path $legacyInstall 'clash-of-rust.exe')) { throw 'Legacy executable was not removed' }
if (-not (Test-Path -LiteralPath (Join-Path $legacyInstall 'user-file.txt'))) { throw 'Migration removed unrelated user data' }
if (-not (Test-Path -LiteralPath (Join-Path $taskInstall 'clash-of-rust.exe'))) { throw 'Migration did not install into the new directory' }
$removed = Start-Process -FilePath $uninstall -ArgumentList '/S',"_?=$taskInstall" -WindowStyle Hidden -Wait -PassThru
if ($removed.ExitCode -ne 0 -or (Test-Path -LiteralPath $testKey)) { throw 'Migrated installation cleanup failed' }
Write-Output 'PASS: fresh installation, offline resources, duplicate refusal, confirmed reinstall, legacy directory migration, uninstallation, preservation of unrelated user files.'
Write-Output "Test artifacts: $taskTestRoot"
