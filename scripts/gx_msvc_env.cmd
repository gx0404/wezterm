@echo off
rem Initialize the installed Visual Studio x64 environment without changing global settings.
rem Usage: scripts\gx_msvc_env.cmd --check
rem        scripts\gx_msvc_env.cmd make check
setlocal
set "REPO=%~dp0.."
for %%I in ("%REPO%") do set "REPO=%%~fI"
if not defined WEZTERM_TOOLCHAIN_ROOT set "WEZTERM_TOOLCHAIN_ROOT=%REPO%\.local\tools"
for %%I in ("%WEZTERM_TOOLCHAIN_ROOT%") do set "WEZTERM_TOOLCHAIN_ROOT=%%~fI"
set "TOOLS=%WEZTERM_TOOLCHAIN_ROOT%"
set "MAKEFLAGS="
set "MFLAGS="
set "GNUMAKEFLAGS="
set "CARGO_TARGET_DIR=%REPO%\target"
set "SCCACHE_DIR=%REPO%\.local\sccache"
set "TMP=%REPO%\.local\tmp"
set "TEMP=%TMP%"
set "TMPDIR=%TMP%"
set "RUSTUP_TOOLCHAIN=1.96.1-x86_64-pc-windows-msvc"
set "PATH=%SystemRoot%\System32;%SystemRoot%;%ProgramFiles%\Git\cmd;%ProgramFiles%\Git\bin;%PATH%"
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" (
  echo [gx_msvc_env] Missing vswhere.exe: install Visual Studio C++ tools and Windows SDK.
  exit /b 1
)
set "VSDEVCMD="
for /f "usebackq delims=" %%P in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSDEVCMD=%%P\Common7\Tools\VsDevCmd.bat"
if not exist "%VSDEVCMD%" (
  echo [gx_msvc_env] Missing VsDevCmd.bat: install Visual Studio C++ tools and Windows SDK.
  exit /b 1
)
call "%VSDEVCMD%" -arch=x64 -host_arch=x64 >nul
if errorlevel 1 (
  echo [gx_msvc_env] VsDevCmd initialization failed.
  exit /b 1
)
for %%T in (make\bin\make.exe venv\Scripts\python.exe nextest\bin\cargo-nextest.exe stylua\bin\stylua.exe lua\bin\lua.exe nasm\bin\nasm.exe perl\perl\bin\perl.exe perl\c\bin\cmake.exe) do (
  if not exist "%TOOLS%\%%T" (
    echo [gx_msvc_env] Missing project tool: %TOOLS%\%%T
    goto :no_tools
  )
)
rem Git's bin/sh launcher prepends its bundled Perl; use the shell directly.
set "SHELL=%ProgramFiles%\Git\usr\bin\sh.exe"
set "SHELL=%SHELL:\=/%"
set "MAKESHELL=%SHELL%"
set "PATH=%TOOLS%\make\bin;%TOOLS%\venv\Scripts;%TOOLS%\venv\bin;%TOOLS%\nextest\bin;%TOOLS%\stylua\bin;%TOOLS%\lua\bin;%TOOLS%\nasm\bin;%TOOLS%\perl\perl\bin;%TOOLS%\perl\c\bin;%PATH%"
set "CFLAGS=%CFLAGS% /utf-8"
set "CXXFLAGS=%CXXFLAGS% /utf-8"
cd /d "%REPO%"
if "%~1"=="--check" goto :check
if "%~1"=="" goto :check
if not exist "%SCCACHE_DIR%" mkdir "%SCCACHE_DIR%"
if not exist "%TMP%" mkdir "%TMP%"
if not exist "%SCCACHE_DIR%" exit /b 1
if not exist "%TMP%" exit /b 1
%*
exit /b %errorlevel%

:check
for %%T in (cl.exe nmake.exe rc.exe cmake.exe rustup.exe cargo.exe sh.exe) do (
  where %%T >nul 2>&1
  if errorlevel 1 (
    echo [gx_msvc_env] Missing %%T in the initialized environment.
    exit /b 1
  )
)
cl /? >nul 2>&1
if errorlevel 1 exit /b 1
nmake /? >nul 2>&1
if errorlevel 1 exit /b 1
rc /? >nul 2>&1
if errorlevel 1 exit /b 1
cmake --version
if errorlevel 1 exit /b 1
rustup run %RUSTUP_TOOLCHAIN% rustc --version
if errorlevel 1 exit /b 1
rustup run %RUSTUP_TOOLCHAIN% cargo --version
if errorlevel 1 exit /b 1
rustup run nightly rustfmt --version
if errorlevel 1 exit /b 1
"%TOOLS%\make\bin\make.exe" --version
if errorlevel 1 exit /b 1
"%TOOLS%\nasm\bin\nasm.exe" -v
if errorlevel 1 exit /b 1
"%TOOLS%\perl\perl\bin\perl.exe" -e "print qq(Perl $^V\n)"
if errorlevel 1 exit /b 1
echo [gx_msvc_env] OK: MSVC, Windows SDK, project tools, Rust 1.96.1 and nightly rustfmt.
exit /b 0

:no_tools
echo [gx_msvc_env] Run scripts/setup_env.sh in Git Bash to install checkout-local tools.
exit /b 1
