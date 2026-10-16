Unicode True
!include "MUI2.nsh"
!include "x64.nsh"
!ifndef VERSION
  !define VERSION "0.3.0"
!endif
Name "MemReduct RS"
OutFile "..\dist\MemReduct-RS-${VERSION}-x64-Setup.exe"
InstallDir "$PROGRAMFILES64\MemReduct RS"
InstallDirRegKey HKLM "Software\MemReductRs" "InstallDir"
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
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "..\LICENSE"
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
  FindWindow $0 "MemReductRs.Window.v1"
  ${If} $0 != 0
    MessageBox MB_ICONEXCLAMATION "请先从托盘菜单退出 MemReduct RS，再安装更新。"
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
