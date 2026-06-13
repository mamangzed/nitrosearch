@echo off
echo ========================================
echo NitroSearch Windows Installer
echo ========================================
echo.

:: Get current directory
set "INSTALL_DIR=%~dp0"
set "EXE_PATH=%INSTALL_DIR%nitro.exe"

:: Check if nitro.exe exists
if not exist "%EXE_PATH%" (
    echo [ERROR] nitro.exe not found in %INSTALL_DIR%
    echo Please extract all files to the same folder.
    pause
    exit /b 1
)

echo [INFO] Found nitro.exe at: %EXE_PATH%
echo.

:: Create silent runner script
echo [INFO] Creating silent runner script...
set "RUNNER_PATH=%INSTALL_DIR%NitroSearch-Server.vbs"

(
echo Set WshShell = CreateObject^("WScript.Shell"^)
echo WshShell.Run """%EXE_PATH%"" start --data-dir ""%INSTALL_DIR%data"" --bind 0.0.0.0:8080", 0, False
echo Set WshShell = Nothing
) > "%RUNNER_PATH%"

echo [OK] Created: %RUNNER_PATH%
echo.

:: Ask user if they want auto-start
echo Do you want NitroSearch to start automatically when Windows boots?
echo.
echo [1] Yes - Install to Startup folder
echo [2] No  - Just create desktop shortcut
echo [3] Skip - Manual start only
echo.
set /p choice="Enter choice (1/2/3): "

if "%choice%"=="1" goto install_startup
if "%choice%"=="2" goto create_shortcut
if "%choice%"=="3" goto done

:install_startup
echo.
echo [INFO] Installing to Windows Startup folder...
set "STARTUP_FOLDER=%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup"
copy "%RUNNER_PATH%" "%STARTUP_FOLDER%\NitroSearch-Server.vbs" >nul
if %errorlevel% equ 0 (
    echo [OK] Installed to: %STARTUP_FOLDER%
    echo [INFO] NitroSearch will auto-start on next Windows boot.
) else (
    echo [ERROR] Failed to copy to Startup folder. Try running as Administrator.
)
goto create_shortcut

:create_shortcut
echo.
echo [INFO] Creating desktop shortcut...
set "DESKTOP=%USERPROFILE%\Desktop"
set "SHORTCUT_PATH=%DESKTOP%\NitroSearch Server.lnk"

:: Create shortcut using PowerShell
powershell -Command "$ws = New-Object -ComObject WScript.Shell; $s = $ws.CreateShortcut('%SHORTCUT_PATH%'); $s.TargetPath = '%RUNNER_PATH%'; $s.WorkingDirectory = '%INSTALL_DIR%'; $s.Description = 'Start NitroSearch Server'; $s.Save()"

if exist "%SHORTCUT_PATH%" (
    echo [OK] Created shortcut: %SHORTCUT_PATH%
    echo [INFO] You can double-click this shortcut to start the server.
) else (
    echo [ERROR] Failed to create desktop shortcut.
)
goto done

:done
echo.
echo ========================================
echo Installation Complete!
echo ========================================
echo.
echo To start server manually:
echo   Double-click: %RUNNER_PATH%
echo   Or run: nitro.exe start
echo.
echo Server will be available at: http://localhost:8080
echo.
echo To stop server:
echo   1. Open Task Manager
echo   2. Find "nitro.exe" process
echo   3. End Task
echo.
pause
