; Uninstall steps Tauri's NSIS template cannot know about, inserted through
; bundle.windows.nsis.installerHooks. Updates (/UPDATE) keep both, like the template does.

${UnStrLoc}

; Launch at login keeps the value name it had before the rename, so upgrades find it; the
; template only removes "${PRODUCTNAME}".
!define LEGACY_AUTOSTART_NAME "Usage Control"

; Take usagectl off PATH and delete the copy the app installed. Reinstalling the app puts it
; back at the next launch.
!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    nsExec::Exec '"$INSTDIR\${MAINBINARYNAME}.exe" --unregister-cli'
    Pop $0
  ${EndIf}
!macroend

; Remove the launch-at-login entry only while it still starts this installation.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    ReadRegStr $R0 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${LEGACY_AUTOSTART_NAME}"
    ${UnStrLoc} $R1 $R0 "$INSTDIR\${MAINBINARYNAME}.exe" ">"
    ${If} $R1 != ""
      DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${LEGACY_AUTOSTART_NAME}"
      DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "${LEGACY_AUTOSTART_NAME}"
    ${EndIf}
  ${EndIf}
!macroend
