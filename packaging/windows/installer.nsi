; Per-user Windows installer for an Otherwise desktop app. Built by build-installer.sh:
;   makensis -WX -INPUTCHARSET UTF8 -DAPP_NSH=<stage>/app.nsh installer.nsi
; Everything app-specific comes from app.nsh (rendered from app.toml by render.py).

!ifndef APP_NSH
  !error "pass -DAPP_NSH=<path to the rendered app.nsh>"
!endif
!include "${APP_NSH}"

Unicode true
ManifestDPIAware true
ManifestSupportedOS all
RequestExecutionLevel user
; Reproducible output: no file times in the data block, one fixed compressor.
SetDateSave off
SetCompressor /SOLID lzma
SetDatablockOptimize on
CRCCheck on

; Code signing slot (see README): -DSIGN_CMD="signtool sign ..." signs the uninstaller while it
; is being built and the finished installer; "%1" is the file.
!ifdef SIGN_CMD
  !uninstfinalize '${SIGN_CMD} "%1"' = 0
  !finalize '${SIGN_CMD} "%1"' = 0
!endif

!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}"
!define APPPATHS_KEY "Software\Microsoft\Windows\CurrentVersion\App Paths\${APP_EXE}"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"

Name "${APP_NAME}"
OutFile "${APP_OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${APP_NAME}"
InstallDirRegKey HKCU "${UNINST_KEY}" "InstallLocation"
BrandingText "${APP_PUBLISHER}"
ShowInstDetails hide
ShowUninstDetails hide

VIProductVersion "${APP_VERSION_QUAD}"
VIFileVersion "${APP_VERSION_QUAD}"
VIAddVersionKey /LANG=1033 "ProductName" "${APP_NAME}"
VIAddVersionKey /LANG=1033 "ProductVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=1033 "FileVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=1033 "FileDescription" "${APP_NAME} Setup"
VIAddVersionKey /LANG=1033 "Comments" "${APP_DESCRIPTION}"
VIAddVersionKey /LANG=1033 "CompanyName" "${APP_PUBLISHER}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "© ${APP_PUBLISHER}"

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

!define MUI_ICON "${APP_ICON}"
!define MUI_UNICON "${APP_ICON}"
!define MUI_ABORTWARNING
!define MUI_COMPONENTSPAGE_NODESC

Var Upgrade

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${APP_LICENSE}"
!insertmacro MUI_PAGE_COMPONENTS
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipDirOnUpgrade
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP_EXE}"
!define MUI_FINISHPAGE_NOREBOOTSUPPORT
!insertmacro MUI_PAGE_FINISH

!define MUI_UNCONFIRMPAGE_TEXT_TOP "$(OW_UN_KEEPDATA)"
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

; The first language is the fallback when the system language is not one of these.
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Spanish"
!insertmacro MUI_LANGUAGE "PortugueseBR"
!insertmacro MUI_LANGUAGE "Portuguese"
!insertmacro MUI_LANGUAGE "French"
!include "${__FILEDIR__}\lang.nsh"
!insertmacro APP_LANGSTRINGS

; ------------------------------------------------------------------ helpers

; Waits until the exe on the stack can be opened for writing (Windows locks a running image).
; Interactive: offers to close it (a normal close request, never a forced kill), then Retry.
; Silent: asks it to close once, then exits with code 2 if it is still running.
!macro OW_ENSURE_CLOSED_FN PREFIX
Function ${PREFIX}OwEnsureClosed
  Exch $R0
  Push $R1
  Push $R2
  ${GetFileName} $R0 $R2
  StrCpy $R1 0
  ${Do}
    ${IfNot} ${FileExists} $R0
      ${Break}
    ${EndIf}
    ClearErrors
    FileOpen $0 $R0 a
    ${IfNot} ${Errors}
      FileClose $0
      ${Break}
    ${EndIf}
    ${If} $R1 == 0
      ${IfNot} ${Silent}
        MessageBox MB_OKCANCEL|MB_ICONEXCLAMATION "$(OW_RUNNING)" /SD IDOK IDOK +2
        Quit
      ${EndIf}
      DetailPrint "taskkill /IM $R2"
      nsExec::Exec 'taskkill /IM "$R2"'
      Pop $0
      Sleep 3000
      StrCpy $R1 1
    ${Else}
      ${If} ${Silent}
        SetErrorLevel 2
        Quit
      ${EndIf}
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(OW_STILL_RUNNING)" IDRETRY +2
      Quit
    ${EndIf}
  ${Loop}
  Pop $R2
  Pop $R1
  Pop $R0
FunctionEnd
!macroend
!insertmacro OW_ENSURE_CLOSED_FN ""
!insertmacro OW_ENSURE_CLOSED_FN "un."

; Deletes KEY only while CMDKEY still runs this install (another app may have taken it over).
!macro OW_DELETE_KEY_IF_OURS KEY CMDKEY EXPECTED
  ReadRegStr $0 HKCU "${CMDKEY}" ""
  ${If} $0 == "${EXPECTED}"
    DeleteRegKey HKCU "${KEY}"
  ${EndIf}
!macroend

; Sets the extension's default ProgID only when no handler exists yet (HKCU or machine-wide).
!macro OW_CLAIM_DEFAULT EXTKEY PROGID
  ReadRegStr $0 HKCU "${EXTKEY}" ""
  ReadRegStr $1 HKCR "${EXTKEY}" ""
  ${If} $0 == ""
  ${AndIf} $1 == ""
    WriteRegStr HKCU "${EXTKEY}" "" "${PROGID}"
  ${EndIf}
!macroend

!macro OW_RELEASE_DEFAULT EXTKEY PROGID
  ReadRegStr $0 HKCU "${EXTKEY}" ""
  ${If} $0 == "${PROGID}"
    DeleteRegValue HKCU "${EXTKEY}" ""
  ${EndIf}
  DeleteRegKey /ifempty HKCU "${EXTKEY}\OpenWithProgids"
  DeleteRegKey /ifempty HKCU "${EXTKEY}"
!macroend

; ------------------------------------------------------------------ install

Section "$(OW_SEC_APP)" SecApp
  SectionIn RO
  SetShellVarContext current
  !insertmacro APP_ENSURE_CLOSED ""

  ; Upgrade in place: the previous uninstaller removes the old files (never the user's data).
  ${If} ${FileExists} "$INSTDIR\uninstall.exe"
    ExecWait '"$INSTDIR\uninstall.exe" /S /UPGRADE _?=$INSTDIR' $0
    Delete "$INSTDIR\uninstall.exe"
  ${EndIf}
  ; The optional sections below put these back if still chosen.
  Delete "$DESKTOP\${APP_NAME}.lnk"
!ifdef APP_AUTOSTART
  !insertmacro APP_RUN_VALUE
  ReadRegStr $1 HKCU "${RUN_KEY}" "${APP_ID}"
  ${If} $1 == $0
    DeleteRegValue HKCU "${RUN_KEY}" "${APP_ID}"
  ${EndIf}
!endif

  !insertmacro APP_INSTALL_FILES
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortcut "$SMPROGRAMS\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_ID}.ico" 0
  !insertmacro APP_EXTRA_SHORTCUTS

  WriteRegStr HKCU "${APPPATHS_KEY}" "" "$INSTDIR\${APP_EXE}"
  WriteRegStr HKCU "${APPPATHS_KEY}" "Path" "$INSTDIR"
  !insertmacro APP_REGISTER

  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "${APP_PUBLISHER}"
  WriteRegStr HKCU "${UNINST_KEY}" "URLInfoAbout" "${APP_URL}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${APP_ID}.ico"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" ${APP_ESTIMATED_SIZE_KB}
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "OwDesktopShortcut" 0
  WriteRegDWORD HKCU "${UNINST_KEY}" "OwAutostart" 0
SectionEnd

!if ${APP_DESKTOP_DEFAULT} == 1
Section "$(OW_SEC_DESKTOP)" SecDesktop
!else
Section /o "$(OW_SEC_DESKTOP)" SecDesktop
!endif
  SetShellVarContext current
  CreateShortcut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_ID}.ico" 0
  WriteRegDWORD HKCU "${UNINST_KEY}" "OwDesktopShortcut" 1
SectionEnd

!ifdef APP_AUTOSTART
!if ${APP_AUTOSTART_DEFAULT} == 1
Section "$(OW_SEC_AUTOSTART)" SecAutostart
!else
Section /o "$(OW_SEC_AUTOSTART)" SecAutostart
!endif
  !insertmacro APP_RUN_VALUE
  WriteRegStr HKCU "${RUN_KEY}" "${APP_ID}" $0
  WriteRegDWORD HKCU "${UNINST_KEY}" "OwAutostart" 1
SectionEnd
!endif

Section "-Notify"
  ; Explorer re-reads associations and context-menu entries.
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd

Function .onInit
  SetShellVarContext current
  StrCpy $Upgrade 0
  ReadRegStr $0 HKCU "${UNINST_KEY}" "InstallLocation"
  ${If} $0 != ""
  ${AndIf} ${FileExists} "$0\${APP_EXE}"
    StrCpy $Upgrade 1
    ; Keep the choices made at the previous install.
    ReadRegDWORD $1 HKCU "${UNINST_KEY}" "OwDesktopShortcut"
    ${If} $1 == 1
      SectionSetFlags ${SecDesktop} ${SF_SELECTED}
    ${Else}
      SectionSetFlags ${SecDesktop} 0
    ${EndIf}
!ifdef APP_AUTOSTART
    ReadRegDWORD $1 HKCU "${UNINST_KEY}" "OwAutostart"
    ${If} $1 == 1
      SectionSetFlags ${SecAutostart} ${SF_SELECTED}
    ${Else}
      SectionSetFlags ${SecAutostart} 0
    ${EndIf}
!endif
  ${EndIf}
FunctionEnd

Function SkipDirOnUpgrade
  ${If} $Upgrade == 1
    Abort
  ${EndIf}
FunctionEnd

; ------------------------------------------------------------------ uninstall

Function un.onInit
  SetShellVarContext current
FunctionEnd

; Removes exactly what the installer wrote. Never touches %USERPROFILE%\.otherwise or app data.
Section "Uninstall"
  SetShellVarContext current
  !insertmacro APP_ENSURE_CLOSED "un."

  !insertmacro APP_UNINSTALL_FILES
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  ${GetParameters} $1
  ClearErrors
  ${GetOptions} $1 "/UPGRADE" $2
  ${If} ${Errors}
    ; A real uninstall (not the upgrade step): also shortcuts and autostart.
    Delete "$SMPROGRAMS\${APP_NAME}.lnk"
    Delete "$DESKTOP\${APP_NAME}.lnk"
    !insertmacro APP_REMOVE_EXTRA_SHORTCUTS
!ifdef APP_AUTOSTART
    !insertmacro APP_RUN_VALUE
    ReadRegStr $1 HKCU "${RUN_KEY}" "${APP_ID}"
    ${If} $1 == $0
      DeleteRegValue HKCU "${RUN_KEY}" "${APP_ID}"
    ${EndIf}
!endif
  ${EndIf}

  !insertmacro APP_UNREGISTER
  DeleteRegKey HKCU "${APPPATHS_KEY}"
  DeleteRegKey HKCU "${UNINST_KEY}"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd
