Unicode True
!include "MUI2.nsh"
!include "x64.nsh"
!ifndef VERSION
  !define VERSION "0.4.0"
!endif
Name "MemReduct RS"
OutFile "..\dist\MemReduct-RS-${VERSION}-x64-Setup.exe"
InstallDir "$PROGRAMFILES64\MemReduct RS"
Var ExistingInstall
Var Updating
RequestExecutionLevel admin
SetCompressor /SOLID lzma
ShowInstDetails show
ShowUninstDetails show
Icon "..\app.ico"
UninstallIcon "..\app.ico"
VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=1033 "ProductName" "MemReduct RS"
VIAddVersionKey /LANG=1033 "FileDescription" "MemReduct RS Setup"
VIAddVersionKey /LANG=1033 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "GPL-3.0-only; MemReduct RS contributors"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\memreduct-rs.exe"
!define MUI_FINISHPAGE_RUN_NOTCHECKED
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfUpdating
!insertmacro MUI_PAGE_WELCOME
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfUpdating
!insertmacro MUI_PAGE_LICENSE "..\LICENSE"
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfUpdating
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "This package requires Windows x64."
    Abort
  ${EndIf}
  SetRegView 64
  StrCpy $Updating 0
  ReadRegStr $ExistingInstall HKLM "Software\MemReductRs" "InstallDir"
  ${If} $ExistingInstall == ""
    ReadRegStr $ExistingInstall HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\MemReductRs" "InstallLocation"
  ${EndIf}
  ${If} $ExistingInstall == ""
    SetRegView 32
    ReadRegStr $ExistingInstall HKLM "Software\MemReductRs" "InstallDir"
    SetRegView 64
  ${EndIf}
  ${If} $ExistingInstall != ""
    IfFileExists "$ExistingInstall\memreduct-rs.exe" 0 no_existing_install
    StrCpy $INSTDIR "$ExistingInstall"
    StrCpy $Updating 1
  ${EndIf}
  no_existing_install:
  FindWindow $0 "MemReductRs.Window.v1"
  ${If} $0 != 0
    ; Request a normal shutdown. A busy cleaner may refuse; never terminate it forcibly.
    SendMessage $0 0x8004 0 0 /TIMEOUT=2000
    StrCpy $1 0
    wait_for_app:
      Sleep 100
      FindWindow $0 "MemReductRs.Window.v1"
      ${If} $0 == 0
        Goto app_closed
      ${EndIf}
      IntOp $1 $1 + 1
      IntCmp $1 30 app_still_running wait_for_app app_still_running
    app_still_running:
      MessageBox MB_ICONEXCLAMATION "程序正在清理或仍有对话框打开，请完成操作并从托盘退出后重试。"
      Abort
  ${EndIf}
  app_closed:
FunctionEnd

Function SkipIfUpdating
  ${If} $Updating == 1
    Abort
  ${EndIf}
FunctionEnd

Section "MemReduct RS" Main
  SetShellVarContext all
  SetOutPath "$INSTDIR"
  File "..\target\release\memreduct-rs.exe"
  File "..\target\release\memreduct-cli.exe"
  File "..\memreduct-rs.example.toml"
  File "..\README.md"
  File "..\LICENSE"
  File "..\NOTICE.md"
  SetOutPath "$INSTDIR\docs"
  File "..\docs\*.md"
  SetOutPath "$INSTDIR"
  CreateDirectory "$SMPROGRAMS\MemReduct RS"
  CreateShortcut "$SMPROGRAMS\MemReduct RS\MemReduct RS.lnk" "$INSTDIR\memreduct-rs.exe"
  CreateShortcut "$DESKTOP\MemReduct RS.lnk" "$INSTDIR\memreduct-rs.exe"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\MemReduct RS\卸载.lnk" "$INSTDIR\Uninstall.exe"
  WriteRegStr HKLM "Software\MemReductRs" "InstallDir" "$INSTDIR"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\MemReductRs" "DisplayName" "MemReduct RS"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\MemReductRs" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\MemReductRs" "Publisher" "sadness-sa"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\MemReductRs" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\MemReductRs" "DisplayIcon" "$INSTDIR\memreduct-rs.exe"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\MemReductRs" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\MemReductRs" "QuietUninstallString" '$\"$INSTDIR\Uninstall.exe$\" /S'
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\MemReductRs" "NoModify" 1
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\MemReductRs" "NoRepair" 1
SectionEnd

Function un.onInit
  SetRegView 64
  FindWindow $0 "MemReductRs.Window.v1"
  ${If} $0 != 0
    MessageBox MB_ICONEXCLAMATION "请先从托盘菜单退出 MemReduct RS，再卸载。"
    Abort
  ${EndIf}
FunctionEnd

Section "Uninstall"
  SetShellVarContext all
  Delete "$DESKTOP\MemReduct RS.lnk"
  Delete "$SMPROGRAMS\MemReduct RS\MemReduct RS.lnk"
  Delete "$SMPROGRAMS\MemReduct RS\卸载.lnk"
  RMDir "$SMPROGRAMS\MemReduct RS"
  Delete "$INSTDIR\memreduct-rs.exe"
  Delete "$INSTDIR\memreduct-cli.exe"
  Delete "$INSTDIR\memreduct-rs.example.toml"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\NOTICE.md"
  Delete "$INSTDIR\docs\*.md"
  RMDir "$INSTDIR\docs"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKLM "Software\MemReductRs"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\MemReductRs"
  ; Per-user settings and logs remain available after reinstalling.
SectionEnd
