; Additions to electron-builder's NSIS installer (`nsis.include` in electron-builder.yml).
;
; `/DISABLEUPDATES` turns the app's updater off for the account it is installed for, as
; `DisableUpdates` under `HKCU\Software\Aspen`, which `src/main/updatePolicy.ts` reads. An
; install without it leaves an earlier choice alone, so an update (which runs the installer again
; without it) keeps updating off. Uninstalling, other than for an update, takes the value away.
; An administrator turns updating off for a whole machine by policy instead
; (`HKLM\Software\Policies\Aspen`).

!include FileFunc.nsh

!macro customInstall
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/DISABLEUPDATES" $R1
  ${IfNot} ${Errors}
    WriteRegDWORD HKCU "Software\Aspen" "DisableUpdates" 1
  ${EndIf}
!macroend

!macro customUnInstall
  ${IfNot} ${isUpdated}
    DeleteRegValue HKCU "Software\Aspen" "DisableUpdates"
    DeleteRegKey /ifempty HKCU "Software\Aspen"
  ${EndIf}
!macroend
