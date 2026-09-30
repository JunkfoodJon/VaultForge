; VaultForge installer (NSIS 3). Built by package.ps1, which stages the files
; and passes /DVERSION /DSTAGE /DICON /DOUTFILE. Don't run makensis on it by hand.
;
; Per-user install: VaultForge never needs Administrator, so it installs to
; %LOCALAPPDATA%\Programs\VaultForge with no UAC prompt, and registers its
; uninstaller under HKCU. Your settings and build history live in
; %APPDATA%\VaultTools and are only removed if you say so.

Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma
RequestExecutionLevel user

!include MUI2.nsh
!include x64.nsh
!include FileFunc.nsh
!include LogicLib.nsh

!ifndef VERSION
  !error "pass /DVERSION=x.y.z (package.ps1 does)"
!endif
!ifndef STAGE
  !error "pass /DSTAGE=<staged folder> (package.ps1 does)"
!endif
!ifndef ICON
  !error "pass /DICON=<path to vaultforge.ico> (package.ps1 does)"
!endif
!ifndef OUTFILE
  !define OUTFILE "VaultForge-${VERSION}-setup.exe"
!endif

!define APP        "VaultForge"
!define EXE        "VaultForge.exe"
!define PUBLISHER  "JunkfoodJon"
!define URL        "https://github.com/JunkfoodJon/VaultForge"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\VaultForge"
!define RUN_KEY    "Software\Microsoft\Windows\CurrentVersion\Run"
!define RUN_VALUE  "VaultForge Hotkey Broker"

Name "${APP} ${VERSION}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${APP}"
InstallDirRegKey HKCU "${UNINST_KEY}" "InstallLocation"
ShowInstDetails show
ShowUninstDetails show
BrandingText "${APP} ${VERSION} - PolyForm Noncommercial 1.0.0"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName"     "${APP}"
VIAddVersionKey "CompanyName"     "${PUBLISHER}"
VIAddVersionKey "LegalCopyright"  "Copyright (c) 2026 JunkfoodJon. PolyForm Noncommercial License 1.0.0."
VIAddVersionKey "FileDescription" "${APP} installer"
VIAddVersionKey "FileVersion"     "${VERSION}"
VIAddVersionKey "ProductVersion"  "${VERSION}"

!define MUI_ICON   "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TITLE "Install ${APP} ${VERSION}"
!define MUI_WELCOMEPAGE_TEXT "VaultForge finds the Rust and Python projects on your PC and builds, runs, tests and lints them with one click.$\r$\n$\r$\nIt installs for your Windows account only and doesn't need Administrator.$\r$\n$\r$\nThis installer is not code-signed yet, which is why Windows called the publisher unknown. You can check the download against SHA256SUMS.txt on the release page."
!define MUI_COMPONENTSPAGE_NODESC
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Start VaultForge now"
!define MUI_FINISHPAGE_LINK "Project page, source code and help"
!define MUI_FINISHPAGE_LINK_LOCATION "${URL}"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${STAGE}\LICENSE.md"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; ── Helpers ──────────────────────────────────────────────────────────

; A visible VaultForge window may be mid-build: ask the user to close it
; rather than killing it. Whatever is left afterwards is the windowless
; Ctrl+Alt+B listener, which holds the exe open and is safe to stop — the
; app starts it again the next time it runs.
!macro STOP_VAULTFORGE UN
Function ${UN}StopVaultForge
  retry:
    nsExec::ExecToStack 'tasklist.exe /NH /FI "IMAGENAME eq ${EXE}" /FI "WINDOWTITLE eq ${APP}"'
    Pop $0
    Pop $1
    StrCpy $2 $1 ${NSIS_MAX_STRLEN}
    ClearErrors
    ${${UN}WordFind} "$2" "${EXE}" "E+1{" $3
    IfErrors listener_only
    MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "VaultForge is open. Close its window and click Retry." /SD IDCANCEL IDRETRY retry
    Abort
  listener_only:
    nsExec::Exec 'taskkill.exe /F /IM "${EXE}"'
    Pop $0
    Sleep 500
FunctionEnd
!macroend
!include WordFunc.nsh
!insertmacro WordFind
!insertmacro un.WordFind
!insertmacro STOP_VAULTFORGE ""
!insertmacro STOP_VAULTFORGE "un."

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "VaultForge needs 64-bit Windows 10 or 11." /SD IDOK
    Abort
  ${EndIf}
FunctionEnd

; ── Install ──────────────────────────────────────────────────────────

Section "VaultForge" SecMain
  SectionIn RO
  Call StopVaultForge
  SetOutPath "$INSTDIR"

  File "${STAGE}\${EXE}"
  File "${STAGE}\README.md"
  File "${STAGE}\LICENSE.md"
  File "${STAGE}\PRIVACY.md"
  File "${STAGE}\SECURITY.md"
  File "${STAGE}\THIRD_PARTY_NOTICES.md"
  SetOutPath "$INSTDIR\licenses\egui-fonts"
  File "${STAGE}\licenses\egui-fonts\*.txt"
  SetOutPath "$INSTDIR"

  SetShellVarContext current
  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\${EXE}" 0 SW_SHOWNORMAL "" "Build, run and test your Rust and Python projects"
  ; A portable copy may have made its own Start Menu entry earlier.
  Delete "$SMPROGRAMS\VaultForge Portable.lnk"

  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr   HKCU "${UNINST_KEY}" "DisplayName"          "${APP}"
  WriteRegStr   HKCU "${UNINST_KEY}" "DisplayVersion"       "${VERSION}"
  WriteRegStr   HKCU "${UNINST_KEY}" "Publisher"            "${PUBLISHER}"
  WriteRegStr   HKCU "${UNINST_KEY}" "DisplayIcon"          "$INSTDIR\${EXE}"
  WriteRegStr   HKCU "${UNINST_KEY}" "InstallLocation"      "$INSTDIR"
  WriteRegStr   HKCU "${UNINST_KEY}" "UninstallString"      "$\"$INSTDIR\uninstall.exe$\""
  WriteRegStr   HKCU "${UNINST_KEY}" "QuietUninstallString" "$\"$INSTDIR\uninstall.exe$\" /S"
  WriteRegStr   HKCU "${UNINST_KEY}" "URLInfoAbout"         "${URL}"
  WriteRegStr   HKCU "${UNINST_KEY}" "HelpLink"             "${URL}"
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" "$0"
SectionEnd

Section /o "Desktop shortcut" SecDesktop
  SetShellVarContext current
  CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\${EXE}" 0
SectionEnd

; ── Uninstall ────────────────────────────────────────────────────────

Section "Uninstall"
  Call un.StopVaultForge

  ; The Ctrl+Alt+B listener's sign-in entry, and the Startup shortcut older
  ; builds used instead.
  DeleteRegValue HKCU "${RUN_KEY}" "${RUN_VALUE}"
  SetShellVarContext current
  Delete "$SMSTARTUP\VaultForge Hotkey Broker.lnk"
  Delete "$SMPROGRAMS\${APP}.lnk"
  Delete "$SMPROGRAMS\VaultForge Portable.lnk"
  Delete "$DESKTOP\${APP}.lnk"

  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE.md"
  Delete "$INSTDIR\PRIVACY.md"
  Delete "$INSTDIR\SECURITY.md"
  Delete "$INSTDIR\THIRD_PARTY_NOTICES.md"
  RMDir /r "$INSTDIR\licenses"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  ; shared-history.json in the same folder belongs to VaultLauncher too, so it
  ; is never removed here.
  MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 "Also delete VaultForge's settings, workspace cache and build logs in$\r$\n$APPDATA\VaultTools ?$\r$\n$\r$\nNo keeps them, so a later install picks up where you left off. Pins and learned build times in shared-history.json are always kept (VaultLauncher uses them too)." /SD IDNO IDNO keep_data
    Delete "$APPDATA\VaultTools\vaultforge-*.json"
    Delete "$APPDATA\VaultTools\vaultforge-*.stamp"
    Delete "$APPDATA\VaultTools\vaultforge-gui.pid"
    RMDir /r "$APPDATA\VaultTools\logs"
    RMDir "$APPDATA\VaultTools"
  keep_data:

  DeleteRegKey HKCU "${UNINST_KEY}"
SectionEnd
