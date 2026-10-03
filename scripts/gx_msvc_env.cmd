@echo off
rem 进入 Visual Studio 2022 Build Tools 的 x64 环境后在仓库根执行给定命令。
rem 用法（任意 cmd / PowerShell）：scripts\gx_msvc_env.cmd python scripts\gx_package.py windows --check
rem
rem 解决的坑：
rem   1) MSYS2 的 git 若排在 PATH 前面，`git rev-parse --show-toplevel` 返回 POSIX 路径，
rem      gx_package.py 会误报「must be an independent Git checkout」——这里把 Git for Windows 提前。
rem   2) vendored OpenSSL 的 MSVC 构建需要 Windows 版 Perl（Strawberry）与 NASM；
rem      两者只认仓内 .local/tools/（make setup 即 scripts/setup_env.sh 钉版安装），
rem      不再读系统路径，MSYS perl 与 Strawberry 自带的旧 nasm 也不会被选中。
rem   3) 仓库路径必须纯 ASCII 是硬前提（非 ASCII 路径会让 Perl/nmake 把 OpenSSL 产物
rem      写进乱码目录，2026-10-03 实测）；追加 /utf-8 与 NASM 3.02 只是双保险。
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
if not exist "%REPO%\.local\tools\nasm\bin\nasm.exe" goto :no_tools
if not exist "%REPO%\.local\tools\perl\perl\bin\perl.exe" goto :no_tools
set "PATH=%REPO%\.local\tools\nasm\bin;%REPO%\.local\tools\perl\perl\bin;%REPO%\.local\tools\perl\c\bin;C:\msys64\mingw64\bin;%PATH%"
if not defined RUSTUP_TOOLCHAIN set "RUSTUP_TOOLCHAIN=1.96.1-x86_64-pc-windows-msvc"
if not defined CARGO_TARGET_DIR set "CARGO_TARGET_DIR=%REPO%\target-gx-msvc"
set "CFLAGS=%CFLAGS% /utf-8"
set "CXXFLAGS=%CXXFLAGS% /utf-8"
cd /d "%REPO%"
%*
exit /b %errorlevel%
:no_tools
echo [gx_msvc_env] 先运行 make setup（scripts/setup_env.sh）安装仓内 NASM/Perl（.local/tools/nasm、.local/tools/perl）
exit /b 1
