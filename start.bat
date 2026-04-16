@echo off
setlocal
cd /d "%~dp0"
echo Resume Manager start script (entry: ui/dashboard.html)
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0start.ps1"
set "ERR=%ERRORLEVEL%"
if not "%ERR%"=="0" (
  echo.
  echo Start failed with exit code %ERR%.
  pause
)
endlocal

