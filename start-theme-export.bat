@echo off
setlocal
cd /d "%~dp0"
echo Resume Manager theme-export launcher
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0start-theme-export.ps1"
set "ERR=%ERRORLEVEL%"
if not "%ERR%"=="0" (
  echo.
  echo Launch failed with exit code %ERR%.
  pause
)
endlocal

