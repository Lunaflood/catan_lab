@echo off
rem Launcher. Keep this file pure ASCII: cmd.exe misparses multi-byte text.
rem The real script is run-online.ps1 (PowerShell handles the encoding correctly).
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0run-online.ps1"
if errorlevel 1 pause
