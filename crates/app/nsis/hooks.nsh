; Служба OptimaX і інсталятор (docs/SERVICE.md, розділ 7).
; Служба встановлюється лише кнопкою в програмі (за згодою); інсталятор її не ставить,
; а лише не заважає: зупиняє перед заміною exe, запускає після, видаляє разом із програмою.
; Якщо служби немає, команди нічого не роблять.
;
; Тека встановлення — лише $PROGRAMFILES64\OptimaX (docs/SERVICE.md, розділ 18): служба говорить
; лише з exe звідти (тека, куди пишуть тільки адміністратори). В іншій теці програма просила б
; UAC при кожному запуску. Тож іншу теку інсталятор не приймає: «Так» — ставимо в Program Files,
; «Ні» — скасування. Файл у UTF-8 з BOM: інакше NSIS прочитає українські рядки як ANSI.

!macro NSIS_HOOK_PREINSTALL
  ${If} $INSTDIR != "$PROGRAMFILES64\${PRODUCTNAME}"
    ${If} $LANGUAGE = 1058
      StrCpy $R9 "OptimaX можна встановити лише в $PROGRAMFILES64\${PRODUCTNAME}: службі OptimaX може говорити лише програма звідти, а в іншій теці вона просила б права адміністратора (UAC) при кожному запуску.$\r$\n$\r$\nТак — встановити в $PROGRAMFILES64\${PRODUCTNAME}.$\r$\nНі — скасувати встановлення."
    ${Else}
      StrCpy $R9 "OptimaX can only be installed to $PROGRAMFILES64\${PRODUCTNAME}: the OptimaX service talks only to the app from there, and from any other folder it would ask for administrator rights (UAC) on every start.$\r$\n$\r$\nYes - install to $PROGRAMFILES64\${PRODUCTNAME}.$\r$\nNo - cancel the installation."
    ${EndIf}
    MessageBox MB_YESNO|MB_ICONEXCLAMATION "$R9" /SD IDYES IDYES optimax_dir_ok
    Abort
    optimax_dir_ok:
    StrCpy $INSTDIR "$PROGRAMFILES64\${PRODUCTNAME}"
    SetOutPath $INSTDIR
  ${EndIf}
  IfFileExists "$INSTDIR\${MAINBINARYNAME}.exe" 0 +2
    nsExec::Exec '"$INSTDIR\${MAINBINARYNAME}.exe" --stop-service'
!macroend

!macro NSIS_HOOK_POSTINSTALL
  nsExec::Exec '"$INSTDIR\${MAINBINARYNAME}.exe" --start-service'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  IfFileExists "$INSTDIR\${MAINBINARYNAME}.exe" 0 +2
    nsExec::Exec '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall-service'
  ; Автозапуск агента в треї (етап 4 docs/SERVICE.md) — лише при справжньому видаленні, не при
  ; оновленні (інакше згода користувача губилася б).
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "OptimaX Agent"
  ${EndIf}
!macroend
