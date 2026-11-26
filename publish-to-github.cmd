@echo off
rem Double-click to commit this project and push it to GitHub.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0publish-to-github.ps1"
pause
