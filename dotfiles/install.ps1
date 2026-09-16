# wezterm-gx installer for Windows (user-level, no admin required).
#
# Deploys the branch-built Windows binaries plus the dotfiles/ snapshot:
#   binaries -> $env:LOCALAPPDATA\Programs\wezterm-gx\<version>\
#   shortcut -> Start Menu "WezTerm (gx)"
#   config   -> $env:USERPROFILE\.config\wezterm\
#   plugins  -> $env:APPDATA\wezterm\plugins\<escaped>\
#   fonts    -> per-user fonts ($env:LOCALAPPDATA\Microsoft\Windows\Fonts + HKCU registry)
#
# Usage (from an elevated-or-not PowerShell, in the extracted bundle root):
#   powershell -ExecutionPolicy Bypass -File dotfiles\install.ps1 [-ZipPath <wezterm-windows.zip>] [-SkipFonts]
#
# Binaries are resolved from: -ZipPath, else .\bin-windows\, else .\bin\.
# Config lookup order in wezterm: %USERPROFILE%\.wezterm.lua wins over
# %USERPROFILE%\.config\wezterm\wezterm.lua -- remove the former if present.

param(
    [string]$ZipPath = "",
    [switch]$SkipFonts
)

$ErrorActionPreference = "Stop"
$BundleRoot = Split-Path -Parent $PSScriptRoot   # dotfiles/.. = bundle root

function Info($msg) { Write-Host "==> $msg" -ForegroundColor Green }
function Warn($msg) { Write-Host "WARN: $msg" -ForegroundColor Yellow }
function Die($msg)  { Write-Host "ERROR: $msg" -ForegroundColor Red; exit 1 }

# ---------------------------------------------------------------- binaries --
$binSrc = $null
if ($ZipPath -ne "") {
    if (-not (Test-Path $ZipPath)) { Die "zip not found: $ZipPath" }
    $staging = Join-Path $env:TEMP "wezterm-gx-staging-$(Get-Random)"
    Expand-Archive -Path $ZipPath -DestinationPath $staging -Force
    $found = Get-ChildItem -Path $staging -Recurse -Filter "wezterm-gui.exe" | Select-Object -First 1
    if (-not $found) { Die "wezterm-gui.exe not found inside $ZipPath" }
    $binSrc = Split-Path -Parent $found.FullName
} elseif (Test-Path (Join-Path $BundleRoot "bin-windows\wezterm-gui.exe")) {
    $binSrc = Join-Path $BundleRoot "bin-windows"
} elseif (Test-Path (Join-Path $BundleRoot "bin\wezterm-gui.exe")) {
    $binSrc = Join-Path $BundleRoot "bin"
} else {
    Die "no binaries found: pass -ZipPath <wezterm-windows.zip> or provide bin-windows\"
}

$version = "dev"
try {
    $version = (& (Join-Path $binSrc "wezterm.exe") --version) | Select-Object -First 1
} catch { Warn "cannot run wezterm.exe to detect version" }
$version = ($version -replace '[\\/:*?"<>| ]', '_')
$dest = Join-Path $env:LOCALAPPDATA "Programs\wezterm-gx\$version"

Info "binaries -> $dest (from $binSrc)"
New-Item -ItemType Directory -Force -Path $dest | Out-Null
Copy-Item -Path (Join-Path $binSrc "*") -Destination $dest -Recurse -Force

# ---------------------------------------------------------------- shortcut --
$startMenu = [Environment]::GetFolderPath("Programs")
$lnk = Join-Path $startMenu "WezTerm (gx).lnk"
Info "start menu shortcut -> $lnk"
$shell = New-Object -ComObject WScript.Shell
$sc = $shell.CreateShortcut($lnk)
$sc.TargetPath = Join-Path $dest "wezterm-gui.exe"
$sc.WorkingDirectory = $env:USERPROFILE
$sc.IconLocation = Join-Path $dest "wezterm-gui.exe,0"
$sc.Save()

# ------------------------------------------------------------------ config --
$cfg = Join-Path $env:USERPROFILE ".config\wezterm"
if (Test-Path $cfg) {
    $bk = "$cfg.bak-gx-$(Get-Date -Format 'yyyyMMdd-HHmmss')"
    Info "backup config -> $bk"
    Copy-Item -Path $cfg -Destination $bk -Recurse
    Remove-Item -Path $cfg -Recurse -Force
}
Info "config -> $cfg"
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $cfg) | Out-Null
Copy-Item -Path (Join-Path $PSScriptRoot "wezterm-config") -Destination $cfg -Recurse
if (Test-Path (Join-Path $env:USERPROFILE ".wezterm.lua")) {
    Warn "$env:USERPROFILE\.wezterm.lua exists and takes precedence over .config\wezterm"
}

# ----------------------------------------------------------------- plugins --
$plugins = Join-Path $env:APPDATA "wezterm\plugins"
Info "plugins -> $plugins"
Get-ChildItem -Path (Join-Path $PSScriptRoot "plugins") -Directory | ForEach-Object {
    $target = Join-Path $plugins $_.Name
    New-Item -ItemType Directory -Force -Path $target | Out-Null
    Copy-Item -Path (Join-Path $_.FullName "*") -Destination $target -Recurse -Force
}

# ------------------------------------------------------------------- fonts --
if (-not $SkipFonts) {
    $fontDir = Join-Path $env:LOCALAPPDATA "Microsoft\Windows\Fonts"
    $reg = "HKCU:\Software\Microsoft\Windows NT\CurrentVersion\Fonts"
    Info "fonts -> $fontDir (per-user registration)"
    New-Item -ItemType Directory -Force -Path $fontDir | Out-Null
    Get-ChildItem -Path (Join-Path $PSScriptRoot "fonts") -Include *.ttf, *.ttc -Recurse | ForEach-Object {
        $target = Join-Path $fontDir $_.Name
        Copy-Item -Path $_.FullName -Destination $target -Force
        $name = [IO.Path]::GetFileNameWithoutExtension($_.Name)
        New-ItemProperty -Path $reg -Name "$name (TrueType)" -Value $target -PropertyType String -Force | Out-Null
    }
} else {
    Info "fonts skipped (-SkipFonts)"
}

Info "installed wezterm-gx $version"
Write-Host ""
Write-Host "Next steps:"
Write-Host "  - launch 'WezTerm (gx)' from the Start Menu"
Write-Host "  - config lives at $env:USERPROFILE\.config\wezterm"
