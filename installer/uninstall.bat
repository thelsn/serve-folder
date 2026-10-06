@echo off
setlocal
rem Removes ServeOn8080: the right-click menu entries and the installed program.
rem Folders that were shared are not touched.

set "APP_DIR=%ProgramFiles%\ServeOn8080"

net session >nul 2>&1
if errorlevel 1 (
    echo Asking for administrator rights...
    set "SELF=%~f0"
    call :elevate
    exit /b
)

echo Removing ServeOn8080...

tasklist /fi "imagename eq serve_folder.exe" 2>nul | find /i "serve_folder.exe" >nul
if not errorlevel 1 (
    echo Stopping running servers...
    taskkill /f /im serve_folder.exe >nul 2>&1
    ping -n 2 127.0.0.1 >nul
)

reg delete "HKCR\Directory\shell\ServeOn8080" /f >nul 2>&1
reg delete "HKCR\Directory\Background\shell\ServeOn8080" /f >nul 2>&1
if exist "%APP_DIR%\serve_folder.exe" del /f /q "%APP_DIR%\serve_folder.exe"
if exist "%APP_DIR%" rmdir "%APP_DIR%" 2>nul

echo.
echo ServeOn8080 has been removed.
echo.
pause
exit /b 0

:elevate
rem Single quotes in the path are doubled for PowerShell
powershell -NoProfile -Command "Start-Process -FilePath '%SELF:'=''%' -Verb RunAs" >nul 2>&1
if errorlevel 1 (
    echo Administrator rights are needed to remove ServeOn8080.
    pause
)
exit /b
