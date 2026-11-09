; Служба OptimaX і інсталятор (docs/SERVICE.md, розділ 7).
; Служба встановлюється лише кнопкою в програмі (за згодою); інсталятор її не ставить,
; а лише не заважає: зупиняє перед заміною exe, запускає після, видаляє разом із програмою.
; Якщо служби немає, команди нічого не роблять.

!macro NSIS_HOOK_PREINSTALL
  IfFileExists "$INSTDIR\${MAINBINARYNAME}.exe" 0 +2
    nsExec::Exec '"$INSTDIR\${MAINBINARYNAME}.exe" --stop-service'
!macroend

!macro NSIS_HOOK_POSTINSTALL
  nsExec::Exec '"$INSTDIR\${MAINBINARYNAME}.exe" --start-service'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  IfFileExists "$INSTDIR\${MAINBINARYNAME}.exe" 0 +2
    nsExec::Exec '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall-service'
!macroend
