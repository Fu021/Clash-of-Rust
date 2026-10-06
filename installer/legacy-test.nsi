; Isolated fixture for testing migration from the former per-user installer.
Unicode true
!include "LogicLib.nsh"
Name "Clash of Rust legacy migration test"
OutFile "${OUTPUT}"
RequestExecutionLevel user
!define LEGACY_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\ClashOfRustLegacySmokeTest"
Section
  SetRegView 64
  SetOutPath "$INSTDIR"
  File "${PAYLOAD}\clash-of-rust.exe"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "${LEGACY_KEY}" "InstallLocation" "$INSTDIR"
SectionEnd
Section "Uninstall"
  SetRegView 64
  Delete "$INSTDIR\clash-of-rust.exe"
  DeleteRegKey HKCU "${LEGACY_KEY}"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
