Unicode true
!include "MUI2.nsh"

Name "Snapline"
OutFile "${APP_ROOT}\dist\Snapline-${APP_VERSION}-Setup.exe"
InstallDir "$LOCALAPPDATA\Programs\Snapline"
InstallDirRegKey HKCU "Software\Snapline\Installer" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
Icon "${APP_ROOT}\assets\snapline.ico"
UninstallIcon "${APP_ROOT}\assets\snapline.ico"

!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\Snapline.exe"
!define MUI_LANGDLL_REGISTRY_ROOT "HKCU"
!define MUI_LANGDLL_REGISTRY_KEY "Software\Snapline\Installer"
!define MUI_LANGDLL_REGISTRY_VALUENAME "Language"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

Function .onInit
    !insertmacro MUI_LANGDLL_DISPLAY
FunctionEnd

Section "Snapline" SEC_APP
    SetShellVarContext current
    SetOutPath "$INSTDIR"
    File "${APP_ROOT}\dist\Snapline.exe"
    File "${APP_ROOT}\README.md"
    File "${APP_ROOT}\LICENSE"
    File "${APP_ROOT}\THIRD_PARTY_NOTICES.md"
    File "${APP_ROOT}\SLINT_LICENSE.md"
    File "${APP_ROOT}\dist\THIRD_PARTY_LICENSES.txt"
    WriteUninstaller "$INSTDIR\Uninstall.exe"
    CreateDirectory "$SMPROGRAMS\Snapline"
    CreateShortcut "$SMPROGRAMS\Snapline\Snapline.lnk" "$INSTDIR\Snapline.exe"
    CreateShortcut "$SMPROGRAMS\Snapline\Uninstall Snapline.lnk" "$INSTDIR\Uninstall.exe"
    WriteRegStr HKCU "Software\Snapline\Installer" "InstallDir" "$INSTDIR"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Snapline" "DisplayName" "Snapline"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Snapline" "DisplayVersion" "${APP_VERSION}"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Snapline" "DisplayIcon" "$INSTDIR\Snapline.exe"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Snapline" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Snapline" "UninstallString" '"$INSTDIR\Uninstall.exe"'
    WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Snapline" "NoModify" 1
    WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Snapline" "NoRepair" 1
SectionEnd

Section "Uninstall"
    SetShellVarContext current
    Delete "$INSTDIR\Snapline.exe"
    Delete "$INSTDIR\README.md"
    Delete "$INSTDIR\LICENSE"
    Delete "$INSTDIR\THIRD_PARTY_NOTICES.md"
    Delete "$INSTDIR\SLINT_LICENSE.md"
    Delete "$INSTDIR\THIRD_PARTY_LICENSES.txt"
    Delete "$INSTDIR\Uninstall.exe"
    RMDir "$INSTDIR"
    Delete "$SMPROGRAMS\Snapline\Snapline.lnk"
    Delete "$SMPROGRAMS\Snapline\Uninstall Snapline.lnk"
    RMDir "$SMPROGRAMS\Snapline"
    DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Snapline"
    DeleteRegKey HKCU "Software\Snapline\Installer"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Snapline"
SectionEnd
