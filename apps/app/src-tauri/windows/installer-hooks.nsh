; NSIS hooks of the Windows installer (Tauri's `installerHooks`).

; Installing, and on each update: the ourstick: address, which opens
; Receive and carries nothing (ADR 0030), for this user.
!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr HKCU "Software\Classes\ourstick" "" "URL:OurStick"
  WriteRegStr HKCU "Software\Classes\ourstick" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\ourstick\DefaultIcon" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\",0"
  WriteRegStr HKCU "Software\Classes\ourstick\shell\open\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
!macroend

; Uninstalling: the File Explorer entries the app made for its sticks
; (ADR 0023), as its lists under HKCU\Software\OurStick\NavigationPane
; name them, one subkey per data folder. Only a braced GUID is taken as an
; entry. Then the start at login (ADR 0024): the Run value and Task
; Manager's switch for it, so that a new install does not inherit an old
; "Disabled"; and the ourstick: address. Not on an update: the old version
; is uninstalled first, and the entries would vanish until the app's next
; start.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    Push $R0
    Push $R1
    Push $R2
    Push $R3
    Push $R4
    StrCpy $R0 0
    ourstick_lists:
      EnumRegKey $R1 HKCU "Software\OurStick\NavigationPane" $R0
      StrCmp $R1 "" ourstick_done
      StrCpy $R2 0
      ourstick_values:
        EnumRegValue $R3 HKCU "Software\OurStick\NavigationPane\$R1" $R2
        StrCmp $R3 "" ourstick_next_list
        StrLen $R4 $R3
        StrCmp $R4 38 0 ourstick_next_value
        StrCpy $R4 $R3 1
        StrCmp $R4 "{" 0 ourstick_next_value
        DeleteRegKey HKCU "Software\Classes\CLSID\$R3"
        DeleteRegKey HKCU "Software\Classes\WOW6432Node\CLSID\$R3"
        DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\Desktop\NameSpace\$R3"
        DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\HideDesktopIcons\NewStartPanel" $R3
      ourstick_next_value:
        IntOp $R2 $R2 + 1
        Goto ourstick_values
    ourstick_next_list:
      IntOp $R0 $R0 + 1
      Goto ourstick_lists
    ourstick_done:
    DeleteRegKey HKCU "Software\OurStick"
    DeleteRegKey HKCU "Software\Classes\ourstick"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "OurStick"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "OurStick"
    Pop $R4
    Pop $R3
    Pop $R2
    Pop $R1
    Pop $R0
  ${EndIf}
!macroend
