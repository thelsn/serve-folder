@echo off
setlocal
rem Installs ServeOn8080: copies serve_folder.exe to Program Files and adds
rem "Host folder on port 8080" to the right-click menu of folders.
rem Running it again updates an existing install.

set "APP_DIR=%ProgramFiles%\ServeOn8080"
set "EXE=%APP_DIR%\serve_folder.exe"
set "SOURCE=%~dp0serve_folder.exe"

rem Program Files and HKEY_CLASSES_ROOT need administrator rights, so ask for them
net session >nul 2>&1
if errorlevel 1 (
    echo Asking for administrator rights...
    set "SELF=%~f0"
    call :elevate
    exit /b
)

echo Installing ServeOn8080...

if not exist "%SOURCE%" (
    echo serve_folder.exe was not found next to install.bat.
    echo Extract the whole zip first, then run install.bat from the extracted folder.
    goto :fail
)

rem A running server keeps the program file locked
tasklist /fi "imagename eq serve_folder.exe" 2>nul | find /i "serve_folder.exe" >nul
if not errorlevel 1 (
    echo Stopping running servers so they can be updated...
    taskkill /f /im serve_folder.exe >nul 2>&1
    ping -n 2 127.0.0.1 >nul
)

if not exist "%APP_DIR%" mkdir "%APP_DIR%" || goto :fail
copy /y "%SOURCE%" "%EXE%" >nul || goto :fail

rem Right-click on a folder
reg add "HKCR\Directory\shell\ServeOn8080" /ve /d "Host folder on port 8080" /f >nul || goto :fail
reg add "HKCR\Directory\shell\ServeOn8080\command" /ve /d "\"%EXE%\" \"%%1\"" /f >nul || goto :fail

rem Right-click on empty space inside a folder
reg add "HKCR\Directory\Background\shell\ServeOn8080" /ve /d "Host this folder on port 8080" /f >nul || goto :fail
reg add "HKCR\Directory\Background\shell\ServeOn8080\command" /ve /d "\"%EXE%\" \"%%V\"" /f >nul || goto :fail

echo.
echo ServeOn8080 is installed in %APP_DIR%
echo.
echo Right-click a folder, or empty space inside one, and choose
echo "Host folder on port 8080". On Windows 11 it is under "Show more options".
echo Then open http://127.0.0.1:8080 in your browser.
echo.
pause
exit /b 0

:fail
echo.
echo Installation failed.
pause
exit /b 1

:elevate
rem Single quotes in the path are doubled for PowerShell
powershell -NoProfile -Command "Start-Process -FilePath '%SELF:'=''%' -Verb RunAs" >nul 2>&1
if errorlevel 1 (
    echo Administrator rights are needed to install ServeOn8080.
    pause
)
exit /b
