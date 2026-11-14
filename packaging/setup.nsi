Unicode true
Name "Kuma Capture"
OutFile "${DEST}"
InstallDir "$LOCALAPPDATA\Programs\Kuma Capture"
RequestExecutionLevel user
SetCompressor zlib
ShowInstDetails show

!include "LogicLib.nsh"
!include "x64.nsh"
!include "FileFunc.nsh"

Page directory
Page instfiles
UninstPage uninstConfirm
UninstPage instfiles

Function .onInit
    ${IfNot} ${RunningX64}
        MessageBox MB_ICONSTOP "Kuma Capture requires 64-bit Windows 11."
        Abort
    ${EndIf}
    System::Call 'kernel32::OpenMutexW(i 0x00100000, i 0, w "Local\KumaCaptureDesktop") p.r0'
    ${If} $0 P<> 0
        System::Call 'kernel32::CloseHandle(p r0)'
        MessageBox MB_ICONSTOP "Quit Kuma Capture from the system tray before updating."
        Abort
    ${EndIf}
    SetRegView 64
    ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kuma Capture" "InstallLocation"
    ${If} $0 == ""
        ReadRegStr $1 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kuma Capture" "UninstallString"
        ${If} $1 != ""
            StrCpy $1 $1 -1 1
            ${GetParent} "$1" $0
        ${EndIf}
    ${EndIf}
    ${If} $0 != ""
    ${AndIf} ${FileExists} "$0\KumaCapture.exe"
        StrCpy $INSTDIR $0
    ${EndIf}
FunctionEnd

Section "Kuma Capture"
    SetOutPath "$PLUGINSDIR"
    File /oname=VC_redist.x64.exe "${VCREDIST}"
    SetRegView 64
    ReadRegDWORD $0 HKLM "SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64" "Installed"
    ReadRegDWORD $1 HKLM "SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64" "Bld"
    ${If} $0 != 1
    ${OrIf} $1 < 33135
        ExecWait '"$PLUGINSDIR\VC_redist.x64.exe" /install /quiet /norestart' $0
        ${If} $0 == 3010
            SetRebootFlag true
        ${ElseIf} $0 != 0
        ${AndIf} $0 != 1638
            MessageBox MB_ICONEXCLAMATION "The Microsoft C++ runtime did not install (code $0). If video capture fails to start, run the included runtime installer from the application folder." /SD IDOK
            SetOutPath "$INSTDIR"
            File /oname=VC_redist.x64.exe "${VCREDIST}"
        ${EndIf}
    ${EndIf}
    SetOutPath "$INSTDIR"
    File /r /x VC_redist.x64.exe "${PAYLOAD}"
    WriteUninstaller "$INSTDIR\Uninstall.exe"
    CreateShortcut "$SMPROGRAMS\Kuma Capture.lnk" "$INSTDIR\KumaCapture.exe"
    CreateShortcut "$SMPROGRAMS\Kuma Capture Updater.lnk" "$INSTDIR\KumaCaptureUpdater.exe"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kuma Capture" "DisplayName" "Kuma Capture"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kuma Capture" "DisplayVersion" "${VERSION}"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kuma Capture" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kuma Capture" "UninstallString" '"$INSTDIR\Uninstall.exe"'
SectionEnd

Section "Uninstall"
    SetRegView 64
    Delete "$SMPROGRAMS\Kuma Capture.lnk"
    Delete "$SMPROGRAMS\Kuma Capture Updater.lnk"
    DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kuma Capture"
    Delete "$INSTDIR\KumaCapture.exe"
    Delete "$INSTDIR\KumaCaptureUpdater.exe"
    RMDir /r "$INSTDIR\sharex"
    Delete "$INSTDIR\VC_redist.x64.exe"
    Delete "$INSTDIR\Uninstall.exe"
    Delete "$INSTDIR\TESTER.md"
    Delete "$INSTDIR\THIRD-PARTY.md"
    Delete "$INSTDIR\ShareX-LICENSE.txt"
    Delete "$INSTDIR\rclone-LICENSE.txt"
    RMDir /r "$INSTDIR\rclone"
    RMDir /r "$INSTDIR\obs\bin"
    RMDir /r "$INSTDIR\obs\core"
    RMDir /r "$INSTDIR\obs\data"
    RMDir /r "$INSTDIR\obs\obs-plugins"
    RMDir "$INSTDIR\obs"
    RMDir "$INSTDIR"
SectionEnd
