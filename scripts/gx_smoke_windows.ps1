# Installer lifecycle tests run only on the disposable GitHub runner.
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Run this install/uninstall test on a disposable GitHub runner.' }
$repo = Split-Path $PSScriptRoot -Parent
$evidence = Join-Path $repo '.ui-evidence/gx-package-windows'
New-Item -ItemType Directory -Force $evidence | Out-Null
Start-Transcript -Path (Join-Path $evidence 'lifecycle.log') | Out-Null
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
        Run-Checked $packages[0].FullName @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/$scope", "/DIR=`"$install`"", "/LOG=`"$evidence/$scope-install.log`"")
        Run-Checked (Join-Path $install 'wezterm-gx-cli.exe') @('--gx-initialize-only')
        $config = Join-Path $env:XDG_CONFIG_HOME 'wezterm/wezterm.lua'
        if (-not (Test-Path -LiteralPath $config)) { throw 'First launch did not seed config' }
        $pluginDirs = @(Get-ChildItem -LiteralPath $plugins -Directory | Where-Object Name -Like 'https*')
        if ($pluginDirs.Count -ne 4) { throw 'Plugin snapshots missing' }
        foreach ($plugin in $pluginDirs) {
            if (-not (Test-Path -LiteralPath (Join-Path $plugin.FullName '.git/HEAD'))) { throw 'gitdir was not restored' }
        }
        $fontLog = & (Join-Path $install 'wezterm.exe') --config-file $config ls-fonts 2>&1
        $fontExit = $LASTEXITCODE
        $fontLog | Set-Content -LiteralPath (Join-Path $evidence "$scope-fonts.log") -Encoding utf8
        if ($fontExit -ne 0) { throw 'Bundled Lua configuration/font smoke failed' }
        if ($fontLog -match 'plugin load failed|Error loading configuration') { throw 'Bundled plugins failed to load' }
        $fontText = $fontLog -join "`n"
        if ($fontText -notmatch 'JetBrainsMono Nerd Font' -or $fontText -notmatch 'Noto Sans CJK SC') {
            throw 'Bundled primary/CJK fonts were not resolved'
        }
        $state = Join-Path $pluginDirs[0].FullName 'state/gx-preserve.json'
        New-Item -ItemType Directory -Force (Split-Path $state) | Out-Null
        Set-Content -LiteralPath $state -Value '{"preserve":true}'
        Add-Content -LiteralPath $config -Value '-- gx-upgrade-preserve-marker'
        $sidecar = Join-Path (Split-Path $config) 'gui-settings.json'
        Set-Content -LiteralPath $sidecar -Value '{"wallpaper":"user wallpaper.png"}'
        $customPlugin = Join-Path $plugins 'user-plugin/plugin/init.lua'
        New-Item -ItemType Directory -Force (Split-Path $customPlugin) | Out-Null
        Set-Content -LiteralPath $customPlugin -Value 'return {}'
        $preserved = @{}
        foreach ($path in @($config, $state, $sidecar, $customPlugin)) {
            $preserved[$path] = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
        }
        Run-Checked (Join-Path $install 'wezterm-gx-cli.exe') @('--gx-initialize-only')
        if (-not (Select-String -LiteralPath $config -Pattern 'gx-upgrade-preserve-marker' -Quiet)) { throw 'Config overwritten' }
        Run-Checked $packages[0].FullName @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/$scope", "/DIR=`"$install`"", "/LOG=`"$evidence/$scope-upgrade.log`"")
        # Exercise plugin replacement, not just the same-version early return.
        $managed = Join-Path $env:APPDATA 'wezterm-gx'
        Set-Content -LiteralPath (Join-Path $managed 'resource-version') -Value 'older' -Encoding ascii
        Run-Checked (Join-Path $install 'wezterm-gx-cli.exe') @('--gx-initialize-only')
        if (-not (Test-Path -LiteralPath (Join-Path $managed 'backups'))) { throw 'Plugin upgrade backup missing' }
        Run-Checked (Join-Path $install 'unins000.exe') @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$evidence/$scope-uninstall.log`"")
        if (-not (Test-Path -LiteralPath $config)) { throw 'Uninstall deleted user configuration' }
        if (-not (Test-Path -LiteralPath $state)) { throw 'Uninstall deleted user session data' }
        if (Test-Path -LiteralPath (Join-Path $install 'wezterm-gx.exe')) { throw 'Uninstall left program files' }
        foreach ($path in $preserved.Keys) {
            if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $preserved[$path]) {
                throw "Upgrade/uninstall modified user data: $path"
            }
        }
        "PASS: $scope; config, wallpaper setting, session and custom plugin hashes preserved" |
            Add-Content -LiteralPath (Join-Path $evidence 'result.txt')
    }
} finally {
    $env:XDG_CONFIG_HOME = $previousXdg
    Stop-Transcript | Out-Null
}
Write-Output 'PASS: current-user/all-users install, initialization, reinstallation and uninstall'
