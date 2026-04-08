@echo off
setlocal

cd /d "%~dp0"

set "LLVM_BIN=D:\Program Files\LLVM\bin"
if exist "%LLVM_BIN%\clang.exe" (
  set "PATH=%LLVM_BIN%;%PATH%"
  set "LIBCLANG_PATH=%LLVM_BIN%"
)

echo [build-win-no-pause] workspace: %CD%
echo [build-win-no-pause] llvm: %LLVM_BIN%
where clang >nul 2>nul
if errorlevel 1 (
  echo [build-win-no-pause] ERROR: clang not found in PATH.
  echo [build-win-no-pause] Fix LLVM path or edit build-win-no-pause.bat.
  exit /b 1
)

call npm run tauri:build:win
set "BUILD_EXIT=%ERRORLEVEL%"

if not "%BUILD_EXIT%"=="0" (
  echo [build-win-no-pause] build failed with exit code %BUILD_EXIT%.
  exit /b %BUILD_EXIT%
)

set "DOWNLOADS_DIR=%USERPROFILE%\Downloads"
set "EXE_SOURCE=%CD%\src-tauri\target\release\codex-monitor.exe"
set "EXE_TARGET=%DOWNLOADS_DIR%\codex-monitor.exe"

if exist "%EXE_SOURCE%" (
  if exist "%EXE_TARGET%" del /f /q "%EXE_TARGET%"
  copy /y "%EXE_SOURCE%" "%EXE_TARGET%" >nul
)

for %%F in ("%CD%\src-tauri\target\release\bundle\nsis\*.exe") do (
  if exist "%%~fF" (
    if exist "%DOWNLOADS_DIR%\%%~nxF" del /f /q "%DOWNLOADS_DIR%\%%~nxF"
    copy /y "%%~fF" "%DOWNLOADS_DIR%\%%~nxF" >nul
  )
)

echo [build-win-no-pause] build finished successfully.
echo [build-win-no-pause] exe: src-tauri\target\release\codex-monitor.exe
echo [build-win-no-pause] installer: src-tauri\target\release\bundle\nsis\
echo [build-win-no-pause] copied to: %DOWNLOADS_DIR%
exit /b 0
