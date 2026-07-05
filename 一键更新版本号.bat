@echo off
setlocal EnableExtensions
chcp 65001 >nul
rem ============================================================
rem  HypoMuxPlus 一键更新版本号
rem  用法:
rem    直接双击运行后按提示输入版本号, 或:
rem    一键更新版本号.bat 1.2.0
rem  流程: 更新所有版本号引用 -> 构建前端 -> 编译 Rust(release) -> 覆盖根目录 HypoMuxPlus.exe
rem ============================================================

rem 切换到脚本所在目录(项目根)
cd /d "%~dp0"

set "NEWVER=%~1"
if "%NEWVER%"=="" set /p "NEWVER=请输入新的版本号 (格式 X.Y.Z, 例如 1.2.0): "

rem 去除可能的首尾空格
for /f "tokens=* delims= " %%a in ("%NEWVER%") do set "NEWVER=%%a"

if "%NEWVER%"=="" (
  echo [错误] 未输入版本号。
  goto :fail
)

echo.
echo ============================================================
echo   目标版本号: %NEWVER%
echo ============================================================
echo.

echo [1/4] 更新版本号引用...
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\update-version.ps1" -NewVersion "%NEWVER%"
if errorlevel 1 (
  echo [错误] 版本号更新失败。
  goto :fail
)

echo.
echo [2/4] 构建前端 (npm run build)...
call npm run build
if errorlevel 1 (
  echo [错误] 前端构建失败。
  goto :fail
)

echo.
echo [3/4] 编译 Rust release (cargo build --release)...
call cargo build --release --manifest-path "src-tauri\Cargo.toml"
if errorlevel 1 (
  echo [错误] Rust 编译失败。
  goto :fail
)

echo.
echo [4/4] 覆盖根目录 HypoMuxPlus.exe...
set "EXE="
if exist "src-tauri\target\release\HypoMuxPlus.exe" set "EXE=src-tauri\target\release\HypoMuxPlus.exe"
if not defined EXE if exist "src-tauri\target\release\hypomuxplus.exe" set "EXE=src-tauri\target\release\hypomuxplus.exe"
if not defined EXE (
  echo [错误] 未找到编译产物 exe (src-tauri\target\release\)。
  goto :fail
)
copy /Y "%EXE%" "HypoMuxPlus.exe" >nul
if errorlevel 1 (
  echo [错误] 复制 exe 到根目录失败。
  goto :fail
)

echo.
echo ============================================================
echo   完成! 版本号已统一更新为 %NEWVER%, 并已重新编译覆盖 HypoMuxPlus.exe
echo ============================================================
endlocal
exit /b 0

:fail
echo.
echo ============================================================
echo   流程中断, 请查看上方错误信息。
echo ============================================================
endlocal
exit /b 1
