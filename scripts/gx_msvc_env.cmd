@echo off
rem 进入 Visual Studio 2022 Build Tools 的 x64 环境后在仓库根执行给定命令。
rem 用法（任意 cmd / PowerShell）：scripts\gx_msvc_env.cmd python scripts\gx_package.py windows --check
rem
rem 解决的坑：
rem   1) MSYS2 的 git 若排在 PATH 前面，`git rev-parse --show-toplevel` 返回 POSIX 路径，
rem      gx_package.py 会误报「must be an independent Git checkout」——这里把 Git for Windows 提前。
rem   2) vendored OpenSSL 的 MSVC 构建需要 Windows 版 Perl（Strawberry）与 NASM >= 3.02
rem      （中文仓库路径的汇编调试信息），MSYS perl / Strawberry 自带的旧 nasm 都不行。
rem   3) 中文仓库路径 + 非 UTF-8 代码页：C/C++ 编译参数追加 /utf-8。
rem   4) 产物目录固定为 target-gx-msvc（已 gitignore），不与 gnu 工具链的 target/ 混用。
setlocal
set "REPO=%~dp0.."
for %%I in ("%REPO%") do set "REPO=%%~fI"
set "PATH=C:\Windows\System32;C:\Windows;C:\Program Files\Git\cmd;C:\Program Files (x86)\Microsoft Visual Studio\Installer;%PATH%"
set "VSDEVCMD=C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\VsDevCmd.bat"
if not exist "%VSDEVCMD%" (
  for /f "usebackq delims=" %%P in (`vswhere.exe -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSDEVCMD=%%P\Common7\Tools\VsDevCmd.bat"
)
if not exist "%VSDEVCMD%" (
  echo [gx_msvc_env] 找不到 VsDevCmd.bat：请安装 Visual Studio 2022 Build Tools（C++ 工作负载 + Windows SDK）
  exit /b 1
)
call "%VSDEVCMD%" -arch=x64 -host_arch=x64 >nul
if errorlevel 1 (
  echo [gx_msvc_env] VsDevCmd 初始化失败
  exit /b 1
)
set "PATH=%LOCALAPPDATA%\bin\NASM;C:\Strawberry\perl\bin;C:\Strawberry\c\bin;C:\msys64\mingw64\bin;%PATH%"
if not defined RUSTUP_TOOLCHAIN set "RUSTUP_TOOLCHAIN=1.96.1-x86_64-pc-windows-msvc"
if not defined CARGO_TARGET_DIR set "CARGO_TARGET_DIR=%REPO%\target-gx-msvc"
set "CFLAGS=%CFLAGS% /utf-8"
set "CXXFLAGS=%CXXFLAGS% /utf-8"
cd /d "%REPO%"
%*
