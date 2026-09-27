# Installer lifecycle tests run only on the disposable GitHub runner.
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Run this install/uninstall test on a disposable GitHub runner.' }
$repo = Split-Path $PSScriptRoot -Parent
$packages = @(Get-ChildItem -LiteralPath (Join-Path $repo 'dist') -Filter 'WezTerm-GX-*-Setup-x64.exe')
if ($packages.Count -ne 1) { throw 'Expected exactly one Windows installer' }
$testRoot = Join-Path $repo ".local/gx-tests/install-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $testRoot | Out-Null
$previousXdg = $env:XDG_CONFIG_HOME
$plugins = Join-Path $env:APPDATA 'wezterm/plugins'
if (Test-Path -LiteralPath $plugins) { throw 'Runner must not already have WezTerm plugin data.' }
# The real program uses Windows Known Folders, not APPDATA/USERPROFILE overrides.
# Use the disposable runner identity for plugins and an isolated config directory.
$env:XDG_CONFIG_HOME = Join-Path $testRoot '用户 config'
function Run-Checked([string]$File, [string[]]$Arguments) {
    $result = Start-Process -FilePath $File -ArgumentList $Arguments -WindowStyle Hidden -Wait -PassThru
    if ($result.ExitCode -ne 0) { throw "$File returned $($result.ExitCode)" }
}
try {
    foreach ($scope in @('CURRENTUSER', 'ALLUSERS')) {
        $install = Join-Path $testRoot $scope
        Run-Checked $packages[0].FullName @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/$scope", "/DIR=`"$install`"")
        Run-Checked (Join-Path $install 'wezterm-gx-cli.exe') @('--gx-initialize-only')
        $config = Join-Path $env:XDG_CONFIG_HOME 'wezterm/wezterm.lua'
        if (-not (Test-Path -LiteralPath $config)) { throw 'First launch did not seed config' }
        $pluginDirs = @(Get-ChildItem -LiteralPath $plugins -Directory)
        if ($pluginDirs.Count -ne 4) { throw 'Plugin snapshots missing' }
        foreach ($plugin in $pluginDirs) {
            if (-not (Test-Path -LiteralPath (Join-Path $plugin.FullName '.git/HEAD'))) { throw 'gitdir was not restored' }
        }
        $fontLog = & (Join-Path $install 'wezterm.exe') --config-file $config ls-fonts 2>&1
        if ($LASTEXITCODE -ne 0) { throw 'Bundled Lua configuration/font smoke failed' }
        if ($fontLog -match 'plugin load failed|Error loading configuration') { throw 'Bundled plugins failed to load' }
        $fontText = $fontLog -join "`n"
        if ($fontText -notmatch 'JetBrainsMono Nerd Font' -or $fontText -notmatch 'Noto Sans CJK SC') {
            throw 'Bundled primary/CJK fonts were not resolved'
        }
        $state = Join-Path $pluginDirs[0].FullName 'state/gx-preserve.json'
        New-Item -ItemType Directory -Force (Split-Path $state) | Out-Null
        Set-Content -LiteralPath $state -Value '{"preserve":true}'
        Add-Content -LiteralPath $config -Value '-- gx-upgrade-preserve-marker'
        Run-Checked (Join-Path $install 'wezterm-gx-cli.exe') @('--gx-initialize-only')
        if (-not (Select-String -LiteralPath $config -Pattern 'gx-upgrade-preserve-marker' -Quiet)) { throw 'Config overwritten' }
        Run-Checked $packages[0].FullName @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/$scope", "/DIR=`"$install`"")
        Run-Checked (Join-Path $install 'wezterm-gx-cli.exe') @('--gx-initialize-only')
        Run-Checked (Join-Path $install 'unins000.exe') @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART')
        if (-not (Test-Path -LiteralPath $config)) { throw 'Uninstall deleted user configuration' }
        if (-not (Test-Path -LiteralPath $state)) { throw 'Uninstall deleted user session data' }
        if (Test-Path -LiteralPath (Join-Path $install 'wezterm-gx.exe')) { throw 'Uninstall left program files' }
    }
} finally {
    $env:XDG_CONFIG_HOME = $previousXdg
}
Write-Output 'PASS: current-user/all-users install, initialization, reinstallation and uninstall'
