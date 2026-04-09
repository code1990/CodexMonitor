@echo off
setlocal

cd /d "%~dp0"

set "LLVM_BIN=D:\Program Files\LLVM\bin"
if exist "%LLVM_BIN%\clang.exe" (
  set "PATH=%LLVM_BIN%;%PATH%"
  set "LIBCLANG_PATH=%LLVM_BIN%"
)

echo [build-parallel-win-no-pause] workspace: %CD%
echo [build-parallel-win-no-pause] llvm: %LLVM_BIN%
where clang >nul 2>nul
if errorlevel 1 (
  echo [build-parallel-win-no-pause] ERROR: clang not found in PATH.
  echo [build-parallel-win-no-pause] Fix LLVM path or edit build-parallel-win-no-pause.bat.
  exit /b 1
)

call npm run tauri:build:parallel:win
set "BUILD_EXIT=%ERRORLEVEL%"

if not "%BUILD_EXIT%"=="0" (
  echo [build-parallel-win-no-pause] build failed with exit code %BUILD_EXIT%.
  exit /b %BUILD_EXIT%
)

set "DOWNLOADS_DIR=%USERPROFILE%\Downloads"
set "EXE_SOURCE=%CD%\src-tauri\target\release\codex-monitor-parallel-test.exe"
set "EXE_TARGET=%DOWNLOADS_DIR%\codex-monitor-parallel-test.exe"

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

for %%F in ("%CD%\src-tauri\target\release\bundle\msi\*.msi") do (
  if exist "%%~fF" (
    if exist "%DOWNLOADS_DIR%\%%~nxF" del /f /q "%DOWNLOADS_DIR%\%%~nxF"
    copy /y "%%~fF" "%DOWNLOADS_DIR%\%%~nxF" >nul
  )
)

echo [build-parallel-win-no-pause] build finished successfully.
echo [build-parallel-win-no-pause] exe: src-tauri\target\release\codex-monitor-parallel-test.exe
echo [build-parallel-win-no-pause] installer: src-tauri\target\release\bundle\nsis\
echo [build-parallel-win-no-pause] msi: src-tauri\target\release\bundle\msi\
echo [build-parallel-win-no-pause] copied to: %DOWNLOADS_DIR%
exit /b 0
