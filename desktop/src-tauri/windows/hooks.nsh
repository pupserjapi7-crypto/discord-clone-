; Nova – Ergänzungen für den NSIS-Installer (Tauri `bundle.windows.nsis.installerHooks`).

!macro NSIS_HOOK_POSTUNINSTALL
  ; Autostart (tauri-plugin-autostart / auto-launch) beim Deinstallieren
  ; entfernen, sonst bleibt ein toter Eintrag im Autostart-Menü.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Nova"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "Nova"
!macroend
