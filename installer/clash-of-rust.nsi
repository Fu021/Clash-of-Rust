Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!include "FileFunc.nsh"

!ifndef APP_VERSION
!define APP_VERSION "0.5.1"
!endif
!ifndef APP_NUMERIC_VERSION
!define APP_NUMERIC_VERSION "0.5.1.0"
!endif
!ifndef APP_ARCH
!define APP_ARCH "x64"
!endif
!ifndef PAYLOAD
  !define PAYLOAD "..\bundle"
!endif
!ifndef OUTPUT
  !define OUTPUT "..\dist\Clash-of-Rust-${APP_VERSION}-windows-${APP_ARCH}-setup.exe"
!endif
!ifdef INSTALLER_TESTING
  !define PRODUCT_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\ClashOfRustInstallerSmokeTest"
  !define LEGACY_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\ClashOfRustLegacySmokeTest"
  !define APP_MUTEX "Local\ClashOfRust.InstallerSmoke"
  !define APP_EXIT_EVENT "Local\ClashOfRust.InstallerSmoke.Exit"
!else
  !define PRODUCT_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\ClashOfRust"
  !define LEGACY_KEY "${PRODUCT_KEY}"
  !define APP_MUTEX "Local\ClashOfRust.Desktop"
  !define APP_EXIT_EVENT "Local\ClashOfRust.Exit"
!endif

!ifdef INSTALLER_TESTING
  !define PRODUCT_HIVE HKCU
  RequestExecutionLevel user
!else
  !define PRODUCT_HIVE HKLM
  RequestExecutionLevel admin
!endif

Name "Clash of Rust"
OutFile "${OUTPUT}"
InstallDir "$PROGRAMFILES64\Clash of Rust"
InstallDirRegKey ${PRODUCT_HIVE} "${PRODUCT_KEY}" "InstallLocation"
SetCompressor /SOLID lzma
SetCompressorDictSize 32
ShowInstDetails show
ShowUninstDetails show
VIProductVersion "${APP_NUMERIC_VERSION}"
VIAddVersionKey /LANG=2052 "ProductName" "Clash of Rust"
VIAddVersionKey /LANG=2052 "FileDescription" "Clash of Rust Windows 安装程序"
VIAddVersionKey /LANG=2052 "FileVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=2052 "ProductVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=2052 "LegalCopyright" "Clash of Rust contributors · GPL-3.0-only"

!define MUI_ABORTWARNING
!define MUI_ICON "${__FILEDIR__}\..\resources\icons\app.ico"
!define MUI_UNICON "${__FILEDIR__}\..\resources\icons\app.ico"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${PAYLOAD}\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!ifndef INSTALLER_TESTING
  !define MUI_FINISHPAGE_RUN "$INSTDIR\clash-of-rust.exe"
  !define MUI_FINISHPAGE_RUN_TEXT "立即启动 Clash of Rust"
  !define MUI_FINISHPAGE_SHOWREADME ""
  !define MUI_FINISHPAGE_SHOWREADME_TEXT "添加快捷方式"
  !define MUI_FINISHPAGE_SHOWREADME_FUNCTION CreateShortcuts
!endif
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH
!insertmacro MUI_LANGUAGE "SimpChinese"

Var ExistingDir
Var ResultCode
Var LegacyInstall
Var KeepAutostart
Var UpdateMode
Var UpdateBackup

!macro CheckRunning PREFIX
  System::Call 'kernel32::OpenMutexW(i 0x100000, i 0, w "${APP_MUTEX}") p.r0'
  ${If} $0 != 0
    System::Call 'kernel32::CloseHandle(p r0)'
    IfSilent ${PREFIX}silent ${PREFIX}interactive
    ${PREFIX}interactive:
      MessageBox MB_OK|MB_ICONEXCLAMATION "Clash of Rust 正在运行。请先退出程序，再重新运行安装或卸载。"
    ${PREFIX}silent:
      SetErrorLevel 3
      Abort
  ${EndIf}
!macroend

Function .onInit
!if "${APP_ARCH}" == "arm64"
  ${IfNot} ${IsNativeARM64}
    MessageBox MB_OK|MB_ICONSTOP "此安装包需要 ARM64 Windows。"
    SetErrorLevel 4
    Abort
  ${EndIf}
!else if "${APP_ARCH}" == "x64"
  ${IfNot} ${RunningX64}
    MessageBox MB_OK|MB_ICONSTOP "此安装包需要 64 位 Windows。"
    SetErrorLevel 4
    Abort
  ${EndIf}
!else
  !error "APP_ARCH must be x64 or arm64"
!endif
  SetRegView 64
  !ifdef INSTALLER_TESTING
    SetShellVarContext current
  !else
    SetShellVarContext all
  !endif
  StrCpy $LegacyInstall 0
  StrCpy $KeepAutostart 0
  StrCpy $UpdateMode 0
  StrCpy $UpdateBackup 0
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/UPDATE" $1
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${EndIf}
  ReadRegStr $ExistingDir ${PRODUCT_HIVE} "${PRODUCT_KEY}" "InstallLocation"
    ${If} $ExistingDir == ""
      ReadRegStr $ExistingDir HKCU "${LEGACY_KEY}" "InstallLocation"
      ${If} $ExistingDir != ""
        StrCpy $LegacyInstall 1
      ${EndIf}
    ${EndIf}
  ${If} $ExistingDir != ""
    ${If} $LegacyInstall == 0
      StrCpy $INSTDIR $ExistingDir
    ${EndIf}
    ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "ClashOfRust"
    ${If} $0 == '"$ExistingDir\clash-of-rust.exe" --background'
      StrCpy $KeepAutostart 1
    ${EndIf}
    ReadRegStr $0 HKCU "Software\ClashOfRust\Autostart" "Executable"
    ${If} $0 == "$ExistingDir\clash-of-rust.exe"
      ; Query the task so an intentionally disabled task stays disabled.
      ExecWait '"$ExistingDir\clash-of-rust.exe" --autostart-status' $0
      ${If} $0 == 0
        StrCpy $KeepAutostart 1
      ${EndIf}
    ${EndIf}
    ; Clicking Update in the client authorizes this specific upgrade.
    ${If} $UpdateMode == 1
      Goto uninstall_existing
    ${EndIf}
    !ifdef INSTALLER_TESTING
      ; Only test builds can simulate accepting the reinstall question silently.
      ${GetParameters} $0
      ClearErrors
      ${GetOptions} $0 "/TESTREINSTALL" $1
      IfErrors test_no_confirmation uninstall_existing
      test_no_confirmation:
    !endif
    ; Ordinary silent reinstallation still requires explicit confirmation.
    IfSilent reinstall_silent reinstall_prompt
    reinstall_silent:
      SetErrorLevel 2
      Abort
    reinstall_prompt:
      MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 "检测到已安装 Clash of Rust。是否先卸载现有版本，再继续安装？$\r$\n$\r$\n订阅和个人设置将保留。选择“否”将取消本次安装。" IDYES uninstall_existing
      SetErrorLevel 2
      Abort
    uninstall_existing:
      Call ShutdownExisting
      ; Defer file removal until the user clicks Install in the wizard.
      Goto init_done
  ${EndIf}
  ${If} $UpdateMode == 1
    ; Never let an update silently turn into a new unrelated installation.
    SetErrorLevel 2
    Abort
  ${EndIf}
  !insertmacro CheckRunning init_
  init_done:
FunctionEnd

Function ShutdownExisting
  ${If} $ExistingDir == ""
    Return
  ${EndIf}
  ; Request normal exit. The GUI restores its proxy and stops its own core.
  ; Never extract/run a second GUI executable or force-kill other processes.
  System::Call 'kernel32::OpenMutexW(i 0x100000, i 0, w "${APP_MUTEX}") p.r0 ?e'
  Pop $1
  ${If} $0 == 0
    ${If} $1 == 2
      Return
    ${EndIf}
    Goto shutdown_failed
  ${EndIf}
  System::Call 'kernel32::CloseHandle(p r0)'
  System::Call 'kernel32::OpenEventW(i 2, i 0, w "${APP_EXIT_EVENT}") p.r0'
  ${If} $0 == 0
    Goto shutdown_failed
  ${EndIf}
  System::Call 'kernel32::SetEvent(p r0) i.r1'
  System::Call 'kernel32::CloseHandle(p r0)'
  ${If} $1 == 0
    Goto shutdown_failed
  ${EndIf}
  StrCpy $2 0
  shutdown_wait:
    System::Call 'kernel32::OpenMutexW(i 0x100000, i 0, w "${APP_MUTEX}") p.r0 ?e'
    Pop $1
    ${If} $0 == 0
      ${If} $1 == 2
        Return
      ${EndIf}
      Goto shutdown_failed
    ${EndIf}
    System::Call 'kernel32::CloseHandle(p r0)'
    Sleep 250
    IntOp $2 $2 + 1
    IntCmp $2 60 shutdown_failed shutdown_wait shutdown_failed
  shutdown_failed:
    IfSilent shutdown_silent shutdown_prompt
    shutdown_prompt:
      MessageBox MB_OK|MB_ICONSTOP "现有客户端未能正常退出。请先从托盘退出程序，再重新运行安装包。未开始卸载或安装。"
    shutdown_silent:
    SetErrorLevel 5
    Abort
FunctionEnd

Function RemoveExisting
  ${If} $ExistingDir != ""
      IfFileExists "$ExistingDir\uninstall.exe" 0 uninstall_failed
      ClearErrors
      ExecWait '"$ExistingDir\uninstall.exe" /S _?=$ExistingDir' $ResultCode
      IfErrors uninstall_failed
      ${If} $ResultCode != 0
        Goto uninstall_failed
      ${EndIf}
      ${If} $LegacyInstall == 1
        ReadRegStr $0 HKCU "${LEGACY_KEY}" "InstallLocation"
      ${Else}
        ReadRegStr $0 ${PRODUCT_HIVE} "${PRODUCT_KEY}" "InstallLocation"
      ${EndIf}
      ${If} $0 != ""
        Goto uninstall_failed
      ${EndIf}
      Delete "$ExistingDir\uninstall.exe"
      RMDir "$ExistingDir"
      Goto removal_done
    uninstall_failed:
      MessageBox MB_OK|MB_ICONSTOP "无法完整卸载现有版本。请退出程序并手动卸载后重试。未开始安装新版本。"
      SetErrorLevel 5
      Abort
  ${EndIf}
  removal_done:
FunctionEnd

Section "Clash of Rust" MainSection
  ${If} $UpdateMode == 0
    Call RemoveExisting
  ${EndIf}
  !insertmacro CheckRunning install_
  ${If} $UpdateMode == 1
    ClearErrors
    Rename "$INSTDIR\clash-of-rust.exe" "$INSTDIR\clash-of-rust.update-backup.exe"
    ${If} ${Errors}
      SetErrorLevel 6
      Abort
    ${EndIf}
    StrCpy $UpdateBackup 1
  ${EndIf}
  ; Extraction failures must not look like a successful update to the helper.
  SetErrorLevel 6
  SetRegView 64
  !ifdef INSTALLER_TESTING
    SetShellVarContext current
  !else
    SetShellVarContext all
  !endif
  ClearErrors
  SetOutPath "$INSTDIR"
  File "${PAYLOAD}\clash-of-rust.exe"
  File "${PAYLOAD}\LICENSE"
  SetOutPath "$INSTDIR\resources"
  File /oname=clash-of-rust-${APP_VERSION}.ico "${__FILEDIR__}\..\resources\icons\app.ico"
  File "${PAYLOAD}\resources\mihomo.exe"
  File "${PAYLOAD}\resources\GeoIP.dat"
  File "${PAYLOAD}\resources\GeoSite.dat"
  File "${PAYLOAD}\resources\Country.mmdb"
  File "${PAYLOAD}\resources\ASN.mmdb"
  File "${PAYLOAD}\resources\geodata.json"
  File "${PAYLOAD}\resources\core.json"
  File "${PAYLOAD}\resources\default.yaml"
  File "${PAYLOAD}\resources\settings-defaults.json"
  File "${PAYLOAD}\resources\THIRD-PARTY-NOTICES.txt"
  File /oname=SourceHanSans-LICENSE.txt "${__FILEDIR__}\..\resources\fonts\LICENSE.txt"
  File /oname=Twemoji-LICENSE.txt "${__FILEDIR__}\..\resources\flags\LICENSE-Twemoji.txt"
  File /oname=Unicode-LICENSE.txt "${__FILEDIR__}\..\resources\flags\LICENSE-Unicode.txt"
  SetOutPath "$INSTDIR\resources\ip-check"
  File "${PAYLOAD}\resources\ip-check\LICENSE"
  File "${PAYLOAD}\resources\ip-check\SOURCE.md"
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  ${If} ${Errors}
    Call RestoreUpdate
    SetErrorLevel 6
    Abort
  ${EndIf}
  !ifndef INSTALLER_TESTING
    ${If} $KeepAutostart == 1
      ; Native helper registers a least-privilege user logon task. It preserves
      ; a Run fallback if scheduling is disabled or unavailable.
      ExecWait '"$INSTDIR\clash-of-rust.exe" --autostart-enable' $0
    ${EndIf}
  !endif
  WriteRegStr ${PRODUCT_HIVE} "${PRODUCT_KEY}" "DisplayName" "Clash of Rust"
  WriteRegStr ${PRODUCT_HIVE} "${PRODUCT_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr ${PRODUCT_HIVE} "${PRODUCT_KEY}" "Publisher" "Clash of Rust contributors"
  WriteRegStr ${PRODUCT_HIVE} "${PRODUCT_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr ${PRODUCT_HIVE} "${PRODUCT_KEY}" "DisplayIcon" "$INSTDIR\resources\clash-of-rust-${APP_VERSION}.ico"
  WriteRegStr ${PRODUCT_HIVE} "${PRODUCT_KEY}" "UninstallString" '$\"$INSTDIR\uninstall.exe$\"'
  WriteRegStr ${PRODUCT_HIVE} "${PRODUCT_KEY}" "QuietUninstallString" '$\"$INSTDIR\uninstall.exe$\" /S'
  WriteRegDWORD ${PRODUCT_HIVE} "${PRODUCT_KEY}" "NoModify" 1
  WriteRegDWORD ${PRODUCT_HIVE} "${PRODUCT_KEY}" "NoRepair" 1
  ${If} ${Errors}
    Call RestoreUpdate
    SetErrorLevel 6
    Abort
  ${EndIf}
  !ifndef INSTALLER_TESTING
    ${If} $UpdateMode == 1
      ; Refresh existing links to use the new version's icon.
      Call CreateShortcuts
    ${EndIf}
  !endif
  ${If} $UpdateBackup == 1
    Delete "$INSTDIR\clash-of-rust.update-backup.exe"
    StrCpy $UpdateBackup 0
  ${EndIf}
  SetErrorLevel 0
SectionEnd

Function RestoreUpdate
  ${If} $UpdateBackup == 1
    Delete "$INSTDIR\clash-of-rust.exe"
    Rename "$INSTDIR\clash-of-rust.update-backup.exe" "$INSTDIR\clash-of-rust.exe"
    StrCpy $UpdateBackup 0
  ${EndIf}
FunctionEnd

Function .onInstFailed
  Call RestoreUpdate
FunctionEnd

Function un.onInit
  SetRegView 64
  !ifdef INSTALLER_TESTING
    SetShellVarContext current
  !else
    SetShellVarContext all
  !endif
  !insertmacro CheckRunning uninit_
FunctionEnd

!ifndef INSTALLER_TESTING
Function CreateShortcuts
  SetShellVarContext all
  CreateDirectory "$SMPROGRAMS\Clash of Rust"
  CreateShortcut "$SMPROGRAMS\Clash of Rust\Clash of Rust.lnk" "$INSTDIR\clash-of-rust.exe" "" "$INSTDIR\resources\clash-of-rust-${APP_VERSION}.ico" 0
  CreateShortcut "$SMPROGRAMS\Clash of Rust\卸载 Clash of Rust.lnk" "$INSTDIR\uninstall.exe" "" "$INSTDIR\resources\clash-of-rust-${APP_VERSION}.ico" 0
  CreateShortcut "$DESKTOP\Clash of Rust.lnk" "$INSTDIR\clash-of-rust.exe" "" "$INSTDIR\resources\clash-of-rust-${APP_VERSION}.ico" 0
  System::Call 'shell32::SHChangeNotify(l 0x08000000, i 0, p 0, p 0)'
FunctionEnd
!endif

Section "Uninstall"
  SetRegView 64
  !ifdef INSTALLER_TESTING
    SetShellVarContext current
  !else
    SetShellVarContext all
  !endif
  ; Delete only files owned by this package. User configuration is elsewhere.
  !ifndef INSTALLER_TESTING
    ReadRegStr $0 HKCU "Software\ClashOfRust\Autostart" "Executable"
    ${If} $0 == "$INSTDIR\clash-of-rust.exe"
      ExecWait '"$INSTDIR\clash-of-rust.exe" --autostart-remove' $0
      ${If} $0 != 0
        MessageBox MB_OK|MB_ICONSTOP "无法清理登录启动任务，请检查任务计划程序服务后重试卸载。"
        SetErrorLevel 6
        Abort
      ${EndIf}
    ${EndIf}
  !endif
  ClearErrors
  Delete "$INSTDIR\clash-of-rust.exe"
  IfErrors uninstall_locked
  Delete "$INSTDIR\resources\mihomo.exe"
  IfErrors uninstall_locked
  Delete "$INSTDIR\resources\GeoIP.dat"
  Delete "$INSTDIR\resources\GeoSite.dat"
  Delete "$INSTDIR\resources\Country.mmdb"
  Delete "$INSTDIR\resources\ASN.mmdb"
  Delete "$INSTDIR\resources\geodata.json"
  Delete "$INSTDIR\resources\core.json"
  Delete "$INSTDIR\resources\default.yaml"
  Delete "$INSTDIR\resources\settings-defaults.json"
  Delete "$INSTDIR\resources\THIRD-PARTY-NOTICES.txt"
  Delete "$INSTDIR\resources\SourceHanSans-LICENSE.txt"
  Delete "$INSTDIR\resources\Twemoji-LICENSE.txt"
  Delete "$INSTDIR\resources\Unicode-LICENSE.txt"
  RMDir /r "$INSTDIR\resources\ip-check"
  Delete "$INSTDIR\resources\clash-of-rust-*.ico"
  RMDir "$INSTDIR\resources"
  Delete "$INSTDIR\LICENSE"
  ; Clean up the README shipped by older versions.
  Delete "$INSTDIR\README.md"
  !ifndef INSTALLER_TESTING
    Delete "$DESKTOP\Clash of Rust.lnk"
    ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "ClashOfRust"
    ${If} $0 == '"$INSTDIR\clash-of-rust.exe" --background'
      DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "ClashOfRust"
    ${EndIf}
    Delete "$SMPROGRAMS\Clash of Rust\Clash of Rust.lnk"
    Delete "$SMPROGRAMS\Clash of Rust\卸载 Clash of Rust.lnk"
    RMDir "$SMPROGRAMS\Clash of Rust"
  !endif
  DeleteRegKey ${PRODUCT_HIVE} "${PRODUCT_KEY}"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  SetErrorLevel 0
  Goto uninstall_done
  uninstall_locked:
    IfSilent uninstall_locked_silent uninstall_locked_prompt
    uninstall_locked_prompt:
      MessageBox MB_OK|MB_ICONSTOP "程序或内核文件正在使用，卸载未完成。请退出 Clash of Rust 后重试。"
    uninstall_locked_silent:
      SetErrorLevel 3
      Abort
  uninstall_done:
SectionEnd
