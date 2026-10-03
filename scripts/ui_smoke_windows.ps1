<#
.SYNOPSIS
  Windows UI 冒烟截图：启动 wezterm-gui、可选打开浮层、抓取窗口并登记 result.json。

.DESCRIPTION
  scripts/ui_smoke.sh（Xvfb + xwd + ffmpeg）的 Windows 等价物：
    1. 以 --always-new-process 启动 wezterm-gui（--config 是全局参数，必须放在 start 之前），
       窗格里输出标记文本 WEZTERM-UI-SMOKE-OK-<时间戳>、一行中文与一行 ANSI 色块；
    2. 按进程 id 轮询主窗口句柄（最多 15 秒）；
    3. 按 -Overlay 发送快捷键 / 右键点击打开浮层；浮层在当前配置下不可达时记 skipped，不算失败；
    4. 抓图：先 PrintWindow(PW_RENDERFULLCONTENT)，得到黑图再临时 TOPMOST|NOACTIVATE 后
       CopyFromScreen（全程不调用 SetForegroundWindow）；
    5. 写入或更新 <Out>/result.json（status=CAPTURED、images_reviewed=false），结束 wezterm 进程。

  键位表：-NoConfig 或配置未禁用默认键时用产品默认键（Ctrl+Shift+P / , / / / M）；配置里
  disable_default_key_bindings = true（dotfiles 快照）时用 GX 配置键位（F2 与 Ctrl+Shift+Space
  leader 层，见 dotfiles/wezterm-config/config/bindings.lua）。-Keymap 可显式覆盖。

  键盘输入只会发给前台窗口：发键前必须确认前台窗口属于本脚本启动的 wezterm 进程，否则记 skipped，
  绝不把按键打到别的程序里。窗口不在前台时可加 -Activate 用 AppActivate 请求前台（默认不抢焦点）。

  结果只是「已抓图」：执行者必须读回每张图核对后，再把 images_reviewed 置 true 并改写 status。

.PARAMETER Out
  批次目录（必填）。相对路径按仓库根解析；建议先 `make evidence TASK=<任务>` 分配。
.PARAMETER Exe
  wezterm-gui.exe 路径，默认 <仓库>/target/release/wezterm-gui.exe。
.PARAMETER ConfigFile
  wezterm.lua，默认 <仓库>/dotfiles/wezterm-config/wezterm.lua。
.PARAMETER NoConfig
  用 -n 基线配置（跳过用户配置），忽略 -ConfigFile。
.PARAMETER Overlay
  none | palette | settings | keybinds | menu | wallpaper | context-menu | confirm
.PARAMETER Label
  文件名前缀；输出为 <Label>.png 或 <Label>-<overlay>.png。
.PARAMETER Keymap
  auto（默认，按配置是否禁用默认键推断）| default | gx。
.PARAMETER Capture
  auto（默认，PrintWindow 黑图再退回屏幕抓取）| printwindow | screen。
.PARAMETER LeaderKey
  F13..F20（wezterm 在 Windows 上只给这几个键映射物理键码）：本机 Ctrl+Shift+Space 被别的程序占为
  全局热键时，用它替换 GX 配置的 leader 键。
.PARAMETER KeepIme
  键控浮层发键前默认把 wezterm 窗口的输入法转换模式切到字母数字（抓图后还原）：输入法在中文模式
  下会把无修饰的字母键（leader 层的 s/k/m/w）吃成拼音组合串，绑定收不到。加此开关则不动输入法。
.PARAMETER ExtraConfig
  额外的 --config 覆盖（每项形如 key=value，Lua 表达式），在 initial_cols/rows 之后、start 之前传入。
.PARAMETER LogFile
  把 wezterm-gui 的 stderr（日志）重定向到该文件，排查按键/配置问题用。
.PARAMETER SettleSeconds
  窗口出现后等待配置/插件加载与首帧渲染的秒数。
.PARAMETER OverlayDelayMs
  打开浮层后到抓图前的等待毫秒数。
.PARAMETER Activate
  窗口不在前台时用 AppActivate 请求前台再发键（默认不抢焦点，此时键控浮层记 skipped）。
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$Exe,
    [string]$ConfigFile,
    [switch]$NoConfig,
    [ValidateSet('none', 'palette', 'settings', 'keybinds', 'menu', 'wallpaper', 'context-menu', 'confirm')]
    [string]$Overlay = 'none',
    [string]$Label = 'before',
    [ValidateSet('auto', 'default', 'gx')][string]$Keymap = 'auto',
    [ValidateSet('auto', 'printwindow', 'screen')][string]$Capture = 'auto',
    [ValidatePattern('^F(1[3-9]|20)$')][string]$LeaderKey,
    [switch]$KeepIme,
    [string[]]$ExtraConfig = @(),
    [string]$LogFile,
    [int]$SettleSeconds = 3,
    [int]$OverlayDelayMs = 500,
    [switch]$Activate
)

$ErrorActionPreference = 'Stop'
$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if ($Label -notmatch '^[\w.-]+$') { throw "Label 只允许字母数字、点、下划线、横线：$Label" }

function Resolve-RepoPath([string]$Path) {
    if ([IO.Path]::IsPathRooted($Path)) { return $Path }
    return (Join-Path $Repo $Path)
}

$OutDir = Resolve-RepoPath $Out
if (-not $Exe) { $Exe = Join-Path $Repo 'target/release/wezterm-gui.exe' }
$Exe = Resolve-RepoPath $Exe
if (-not (Test-Path -LiteralPath $Exe -PathType Leaf)) {
    throw "缺少 $Exe；先构建（make build）或用 -Exe 指定 wezterm-gui.exe"
}
if (-not $NoConfig) {
    if (-not $ConfigFile) { $ConfigFile = Join-Path $Repo 'dotfiles/wezterm-config/wezterm.lua' }
    $ConfigFile = Resolve-RepoPath $ConfigFile
    if (-not (Test-Path -LiteralPath $ConfigFile -PathType Leaf)) { throw "缺少配置文件 $ConfigFile" }
}
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

if (-not ('SmokeWin32' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class SmokeWin32 {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx; public int dy; public uint mouseData; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Explicit)] struct INPUTUNION { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public INPUTUNION u; }

    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern uint MapVirtualKey(uint code, uint type);
    [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);
    [DllImport("imm32.dll")] static extern IntPtr ImmGetDefaultIMEWnd(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr SendMessageTimeout(IntPtr h, uint msg, IntPtr w, IntPtr l, uint flags, uint timeout, out IntPtr result);

    // 输入法转换模式：mode < 0 只读，否则设置（0 = 字母数字）；返回设置/读取前的值，失败返回 -1。
    public static int ImeConversion(IntPtr h, int mode) {
        IntPtr ime = ImmGetDefaultIMEWnd(h);
        if (ime == IntPtr.Zero) { return -1; }
        IntPtr old;
        if (SendMessageTimeout(ime, 0x283, (IntPtr)1, IntPtr.Zero, 2, 500, out old) == IntPtr.Zero) { return -1; }   // WM_IME_CONTROL, IMC_GETCONVERSIONMODE
        if (mode >= 0) {
            IntPtr ignore;
            SendMessageTimeout(ime, 0x283, (IntPtr)2, (IntPtr)mode, 2, 500, out ignore);                            // IMC_SETCONVERSIONMODE
        }
        return (int)old;
    }

    // 进程的可见顶层窗口里面积最大的一个（wezterm 的主窗口）。
    public static IntPtr FindWindow(uint pid) {
        IntPtr best = IntPtr.Zero;
        long bestArea = 0;
        EnumWindows(delegate(IntPtr h, IntPtr l) {
            uint p;
            GetWindowThreadProcessId(h, out p);
            if (p == pid && IsWindowVisible(h)) {
                RECT r;
                GetWindowRect(h, out r);
                long area = (long)(r.R - r.L) * (r.B - r.T);
                if (area > bestArea) { best = h; bestArea = area; }
            }
            return true;
        }, IntPtr.Zero);
        return best;
    }

    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, System.Text.StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, System.Text.StringBuilder s, int n);

    // 前台窗口的 pid|类名|标题，用来说明是谁抢走了焦点。
    public static string ForegroundInfo() {
        IntPtr h = GetForegroundWindow();
        System.Text.StringBuilder t = new System.Text.StringBuilder(256);
        System.Text.StringBuilder c = new System.Text.StringBuilder(256);
        GetWindowText(h, t, 256);
        GetClassName(h, c, 256);
        uint p = 0;
        if (h != IntPtr.Zero) { GetWindowThreadProcessId(h, out p); }
        return p + "|" + c + "|" + t;
    }

    public static uint ForegroundPid() {
        uint p = 0;
        IntPtr h = GetForegroundWindow();
        if (h != IntPtr.Zero) { GetWindowThreadProcessId(h, out p); }
        return p;
    }

    // 可见边框（不含 Win10/11 的不可见阴影边）；DWM 不可用时退回 GetWindowRect。
    public static RECT FrameBounds(IntPtr h) {
        RECT r;
        if (DwmGetWindowAttribute(h, 9, out r, Marshal.SizeOf(typeof(RECT))) != 0) { GetWindowRect(h, out r); }
        return r;
    }

    // 单个键盘事件；带 wScan 让 wezterm 的 ToUnicodeEx 能还原字符。
    public static void Key(ushort vk, bool up) {
        INPUT[] a = new INPUT[1];
        a[0].type = 1;
        a[0].u.ki.wVk = vk;
        a[0].u.ki.wScan = (ushort)MapVirtualKey(vk, 0);
        a[0].u.ki.dwFlags = up ? 2u : 0u;
        if (SendInput(1, a, Marshal.SizeOf(typeof(INPUT))) != 1) { throw new System.ComponentModel.Win32Exception(); }
    }

    // 非零像素占比：PrintWindow 失败时典型表现是整幅纯黑。
    public static double NonBlackRatio(byte[] bgra, int stride, int w, int h) {
        long nonzero = 0, total = (long)w * h;
        for (int y = 0; y < h; y++) {
            int row = y * stride;
            for (int x = 0; x < w; x++) {
                int i = row + x * 4;
                if (bgra[i] != 0 || bgra[i + 1] != 0 || bgra[i + 2] != 0) { nonzero++; }
            }
        }
        return total == 0 ? 0.0 : (double)nonzero / total;
    }

    // 最后 rows 行里近白像素（三通道都 > 240）的占比：PrintWindow 偶尔会在窗口底部留一条未绘制的白带。
    public static double BottomWhiteRatio(byte[] bgra, int stride, int w, int h, int rows) {
        long white = 0, total = 0;
        for (int y = Math.Max(0, h - rows); y < h; y++) {
            int row = y * stride;
            for (int x = 0; x < w; x++) {
                int i = row + x * 4;
                total++;
                if (bgra[i] > 240 && bgra[i + 1] > 240 && bgra[i + 2] > 240) { white++; }
            }
        }
        return total == 0 ? 0.0 : (double)white / total;
    }
}
'@
}
Add-Type -AssemblyName System.Drawing
[SmokeWin32]::SetProcessDPIAware() | Out-Null

# ---------- 键位表 ----------
$CTRL = 0xA2; $SHIFT = 0xA0
# GX 配置的 leader 默认是 Ctrl+Shift+Space；有的机器上它是别的程序的全局热键（会抢走前台），
# 此时用 -LeaderKey 改成无修饰键的 F13..F24 经 --config leader=... 覆盖，leader 层绑定本身不变。
if ($LeaderKey) {
    $Leader = @{ m = @(); k = (0x7C + [int]$LeaderKey.Substring(1) - 13) }
}
else {
    $Leader = @{ m = @($CTRL, $SHIFT); k = 0x20 }
}
$KeyMaps = @{
    default = @{
        palette  = @(@{ m = @($CTRL, $SHIFT); k = 0x50 })
        settings = @(@{ m = @($CTRL, $SHIFT); k = 0xBC })
        keybinds = @(@{ m = @($CTRL, $SHIFT); k = 0xBF })
        menu     = @(@{ m = @($CTRL, $SHIFT); k = 0x4D })
        confirm  = @(@{ m = @($CTRL, $SHIFT); k = 0x57 })
    }
    gx      = @{
        palette   = @(@{ m = @(); k = 0x71 })
        settings  = @($Leader, @{ m = @(); k = 0x53 })
        keybinds  = @($Leader, @{ m = @(); k = 0x4B })
        menu      = @($Leader, @{ m = @(); k = 0x4D })
        wallpaper = @($Leader, @{ m = @(); k = 0x57 })
        confirm   = @(@{ m = @($CTRL, $SHIFT); k = 0x57 })
    }
}

function Resolve-Keymap {
    if ($Keymap -ne 'auto') { return $Keymap }
    if ($NoConfig) { return 'default' }
    $files = @($ConfigFile)
    $bindings = Join-Path (Split-Path -Parent $ConfigFile) 'config/bindings.lua'
    if (Test-Path -LiteralPath $bindings) { $files += $bindings }
    foreach ($f in $files) {
        $text = Get-Content -Raw -Encoding UTF8 -LiteralPath $f
        if ($text -match '(?m)^\s*disable_default_key_bindings\s*=\s*true') { return 'gx' }
    }
    return 'default'
}

function Send-Chord($Mods, [int]$Vk) {
    $mods = @($Mods)
    foreach ($m in $mods) { [SmokeWin32]::Key([uint16]$m, $false); Start-Sleep -Milliseconds 25 }
    [SmokeWin32]::Key([uint16]$Vk, $false); Start-Sleep -Milliseconds 50
    [SmokeWin32]::Key([uint16]$Vk, $true); Start-Sleep -Milliseconds 25
    for ($i = $mods.Count - 1; $i -ge 0; $i--) { [SmokeWin32]::Key([uint16]$mods[$i], $true); Start-Sleep -Milliseconds 25 }
}

# ---------- result.json ----------
function ConvertTo-Ordered($Value) {
    if ($Value -is [System.Management.Automation.PSCustomObject]) {
        $o = [ordered]@{}
        foreach ($p in $Value.PSObject.Properties) { $o[$p.Name] = ConvertTo-Ordered $p.Value }
        return $o
    }
    if ($Value -is [System.Collections.IEnumerable] -and $Value -isnot [string]) {
        return , @($Value | ForEach-Object { ConvertTo-Ordered $_ })
    }
    return $Value
}

function Read-Git([string[]]$GitArgs) {
    try { return (& git -C $Repo @GitArgs 2>$null | Select-Object -First 1) } catch { return $null }
}

function Write-Result($Shot, $Skip) {
    $path = Join-Path $OutDir 'result.json'
    $r = [ordered]@{}
    if (Test-Path -LiteralPath $path) {
        $r = ConvertTo-Ordered (Get-Content -Raw -Encoding UTF8 -LiteralPath $path | ConvertFrom-Json)
    }
    if (-not $r.Contains('task')) { $r['task'] = 'ui-smoke' }
    if (-not $r.Contains('branch')) { $b = Read-Git @('branch', '--show-current'); $r['branch'] = if ($b) { $b } else { 'no-git' } }
    if (-not $r.Contains('commit')) { $c = Read-Git @('rev-parse', '--short', 'HEAD'); $r['commit'] = if ($c) { $c } else { 'no-git' } }
    $r['command'] = 'scripts/ui_smoke_windows.ps1'
    $shots = @(); if ($r.Contains('screenshots')) { $shots = @($r['screenshots']) }
    $captures = @(); if ($r.Contains('captures')) { $captures = @($r['captures']) }
    $skipped = @(); if ($r.Contains('skipped')) { $skipped = @($r['skipped']) }
    if ($Shot) {
        if ($shots -notcontains $Shot.file) { $shots += $Shot.file }
        $captures = @($captures | Where-Object { $_.file -ne $Shot.file }) + $Shot
    }
    if ($Skip) { $skipped += $Skip }
    $r['screenshots'] = @($shots)
    $r['captures'] = @($captures)
    if ($skipped.Count -gt 0) { $r['skipped'] = @($skipped) }
    # 新增截图后一律回到「未读回」：旧的 PASS/true 不覆盖新图。
    if ($shots.Count -gt 0) {
        $r['status'] = 'CAPTURED'
        $r['images_reviewed'] = $false
        $r['note'] = '截图已落盘但尚未读回；执行者必须逐张读回，核对有字、有色、无花屏、浮层确实打开后，把 images_reviewed 置 true 并把 status 改 PASS/FAIL'
    }
    else {
        if (-not $r.Contains('status')) { $r['status'] = 'PENDING' }
        if (-not $r.Contains('images_reviewed')) { $r['images_reviewed'] = $false }
        if (-not $r.Contains('note')) { $r['note'] = '尚无截图（浮层不可达或未抓取）；见 skipped' }
    }
    $json = ($r | ConvertTo-Json -Depth 8) + "`n"
    [IO.File]::WriteAllText($path, $json, (New-Object Text.UTF8Encoding $false))
}

# ---------- 抓图 ----------
function Get-Shot([IntPtr]$H, [string]$Mode) {
    $defect = $null
    $frame = [SmokeWin32]::FrameBounds($H)
    $w = $frame.R - $frame.L; $ht = $frame.B - $frame.T
    if ($w -le 0 -or $ht -le 0) { throw "窗口矩形无效 ${w}x${ht}" }
    if ($Mode -ne 'screen') {
        $win = New-Object SmokeWin32+RECT
        [SmokeWin32]::GetWindowRect($H, [ref]$win) | Out-Null
        $full = New-Object System.Drawing.Bitmap(($win.R - $win.L), ($win.B - $win.T))
        $g = [System.Drawing.Graphics]::FromImage($full)
        $hdc = $g.GetHdc()
        $ok = [SmokeWin32]::PrintWindow($H, $hdc, 2)   # PW_RENDERFULLCONTENT
        $g.ReleaseHdc($hdc); $g.Dispose()
        $crop = New-Object System.Drawing.Rectangle(($frame.L - $win.L), ($frame.T - $win.T), $w, $ht)
        $crop.Intersect((New-Object System.Drawing.Rectangle(0, 0, $full.Width, $full.Height)))
        $bmp = $full.Clone($crop, $full.PixelFormat)
        $full.Dispose()
        $data = $bmp.LockBits((New-Object System.Drawing.Rectangle(0, 0, $bmp.Width, $bmp.Height)),
            [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $bytes = New-Object byte[] ($data.Stride * $data.Height)
        [Runtime.InteropServices.Marshal]::Copy($data.Scan0, $bytes, 0, $bytes.Length)
        $bmp.UnlockBits($data)
        $ratio = [SmokeWin32]::NonBlackRatio($bytes, $data.Stride, $bmp.Width, $bmp.Height)
        $white = [SmokeWin32]::BottomWhiteRatio($bytes, $data.Stride, $bmp.Width, $bmp.Height, 12)
        $defect = $null
        if (-not $ok -or $ratio -le 0.005) { $defect = "黑图（ok=$ok 非零像素占比 $ratio）" }
        elseif ($white -gt 0.6) { $defect = "底部残留白带（最后 12 行近白占比 $white）" }
        if (-not $defect) { return @{ bmp = $bmp; method = 'printwindow' } }
        $bmp.Dispose()
        if ($Mode -eq 'printwindow') { throw "PrintWindow 结果无效：$defect" }
        Write-Host "PrintWindow $defect，改用临时 TOPMOST + CopyFromScreen"
    }
    # 临时置顶但不激活，抓屏后恢复；不调用 SetForegroundWindow。
    $flags = [uint32](0x0001 -bor 0x0002 -bor 0x0010 -bor 0x0040)   # NOSIZE|NOMOVE|NOACTIVATE|SHOWWINDOW
    $top = New-Object IntPtr(-1); $notop = New-Object IntPtr(-2)
    [SmokeWin32]::SetWindowPos($H, $top, 0, 0, 0, 0, $flags) | Out-Null
    try {
        Start-Sleep -Milliseconds 800
        $frame = [SmokeWin32]::FrameBounds($H)
        $w = $frame.R - $frame.L; $ht = $frame.B - $frame.T
        $bmp = New-Object System.Drawing.Bitmap($w, $ht)
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen($frame.L, $frame.T, 0, 0, $bmp.Size)
        $g.Dispose()
        return @{ bmp = $bmp; method = 'screen'; fallback = $defect }
    }
    finally {
        [SmokeWin32]::SetWindowPos($H, $notop, 0, 0, 0, 0, $flags) | Out-Null
    }
}

function Quote-Arg([string]$s) {
    if ($s -match '[\s"]') { return '"' + ($s -replace '"', '\"') + '"' }
    return $s
}

# ---------- 启动 ----------
$stamp = (Get-Date).ToUniversalTime().ToString('yyyyMMddHHmmss')
$marker = "WEZTERM-UI-SMOKE-OK-$stamp"
# confirm 需要窗格里有非 shell 的前台进程（pwsh.exe 在 skip_close_confirmation_for_processes_named 默认名单内）。
# ANSI 色块走 [Console]::Out：pwsh 7 在 NO_COLOR 等环境下 $PSStyle.OutputRendering 会降为 PlainText，
# 经 Write-Host 输出的转义序列会被剥掉，验证不到颜色渲染。
$tail = if ($Overlay -eq 'confirm') { 'ping.exe -n 120 127.0.0.1 | Out-Null' } else { 'Start-Sleep 120' }
$inner = @"
`$e = [char]27
Write-Host '$marker'
Write-Host ''
Write-Host 'UI 冒烟 中文渲染'
[Console]::Out.WriteLine("`$e[31m红色`$e[0m `$e[32m绿色`$e[0m `$e[34m蓝色`$e[0m `$e[1m粗体`$e[0m `$e[7m反显`$e[0m")
Write-Host '$marker'
$tail
"@
$shell = (Get-Command pwsh -ErrorAction SilentlyContinue).Source
if (-not $shell) { $shell = (Get-Command powershell -ErrorAction Stop).Source }
$encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($inner))

$parts = @()
if ($NoConfig) { $parts += '-n' } else { $parts += '--config-file'; $parts += (Quote-Arg $ConfigFile) }
$parts += @('--config', 'initial_cols=160', '--config', 'initial_rows=42')
if ($LeaderKey) {
    $parts += @('--config', "leader={key='$LeaderKey',mods='NONE',timeout_milliseconds=2000}")
}
foreach ($kv in $ExtraConfig) { $parts += @('--config', (Quote-Arg $kv)) }
$parts += @('start', '--always-new-process', '--', (Quote-Arg $shell), '-NoLogo', '-NoProfile', '-EncodedCommand', $encoded)
$argString = $parts -join ' '

$proc = $null
$h = [IntPtr]::Zero
$imeBefore = -1
try {
    Write-Host "启动 $Exe（overlay=$Overlay，输出 $OutDir）"
    $spArgs = @{ FilePath = $Exe; ArgumentList = $argString; WorkingDirectory = $Repo; PassThru = $true }
    if ($LogFile) { $spArgs['RedirectStandardError'] = (Resolve-RepoPath $LogFile) }
    $proc = Start-Process @spArgs
    $h = [IntPtr]::Zero
    $deadline = (Get-Date).AddSeconds(15)
    while ((Get-Date) -lt $deadline) {
        if ($proc.HasExited) { throw "wezterm-gui 提前退出（exit=$($proc.ExitCode)），检查配置/字体/显卡" }
        $h = [SmokeWin32]::FindWindow([uint32]$proc.Id)
        if ($h -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 200
    }
    if ($h -eq [IntPtr]::Zero) { throw '15 秒内没有出现 wezterm 主窗口' }
    if ([SmokeWin32]::IsIconic($h)) { [SmokeWin32]::ShowWindow($h, 9) | Out-Null }   # SW_RESTORE
    Start-Sleep -Seconds $SettleSeconds   # 等配置/插件加载与首帧渲染

    $keymap = Resolve-Keymap
    $skip = $null
    $suffix = if ($Overlay -eq 'none') { '' } else { "-$Overlay" }
    $file = "$Label$suffix.png"

    if ($Overlay -eq 'context-menu') {
        # 窗格中心的右键按下（只发 Down：菜单在 Down 时弹出，Up 可能落到弹出的菜单项上）。
        $rc = New-Object SmokeWin32+RECT
        [SmokeWin32]::GetClientRect($h, [ref]$rc) | Out-Null
        $x = [int]($rc.R / 2); $y = [int]($rc.B / 2)
        $lp = [IntPtr](($y -shl 16) -bor ($x -band 0xFFFF))
        [SmokeWin32]::PostMessage($h, 0x0200, [IntPtr]::Zero, $lp) | Out-Null   # WM_MOUSEMOVE
        Start-Sleep -Milliseconds 100
        [SmokeWin32]::PostMessage($h, 0x0204, [IntPtr]2, $lp) | Out-Null        # WM_RBUTTONDOWN, MK_RBUTTON
        Start-Sleep -Milliseconds $OverlayDelayMs
    }
    elseif ($Overlay -ne 'none') {
        $steps = $KeyMaps[$keymap][$Overlay]
        if (-not $steps) {
            $skip = "键位表 $keymap 没有 $Overlay 的绑定，浮层不可达"
        }
        else {
            if ([SmokeWin32]::ForegroundPid() -ne $proc.Id -and $Activate) {
                (New-Object -ComObject WScript.Shell).AppActivate($proc.Id) | Out-Null
                Start-Sleep -Milliseconds 400
            }
            if ([SmokeWin32]::ForegroundPid() -ne $proc.Id) {
                $skip = '前台窗口不属于本次启动的 wezterm，拒绝向其它程序发键（可加 -Activate）'
            }
            else {
                if (-not $KeepIme) {
                    $imeBefore = [SmokeWin32]::ImeConversion($h, 0)
                    Start-Sleep -Milliseconds 200
                }
                $n = 0
                foreach ($step in $steps) {
                    $n++
                    # 每一步前都重新确认前台：若上一步触发了全局热键把焦点抢走，后续按键会落进别的程序。
                    if ([SmokeWin32]::ForegroundPid() -ne $proc.Id) {
                        $skip = "第 $n 步前 wezterm 失去前台（当前前台 $([SmokeWin32]::ForegroundInfo())，疑似全局热键抢焦点），中止发键"
                        break
                    }
                    Send-Chord $step.m $step.k
                    Start-Sleep -Milliseconds 150
                }
                if (-not $skip) { Start-Sleep -Milliseconds $OverlayDelayMs }
            }
        }
    }

    if ($skip) {
        Write-Host "SKIPPED overlay=${Overlay}：$skip"
        Write-Result $null ([ordered]@{
                overlay = $Overlay; label = $Label; reason = $skip; keymap = $keymap
                utc = (Get-Date).ToUniversalTime().ToString('o')
            })
    }
    else {
        $foreground = [SmokeWin32]::ForegroundPid() -eq $proc.Id
        $shot = Get-Shot $h $Capture
        $path = Join-Path $OutDir $file
        $shot.bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
        $size = "$($shot.bmp.Width)x$($shot.bmp.Height)"
        $shot.bmp.Dispose()
        Write-Host "SAVED $path $size method=$($shot.method) keymap=$keymap"
        Write-Result ([ordered]@{
                file = $file; overlay = $Overlay; label = $Label; method = $shot.method; fallback = $shot.fallback; keymap = $keymap
                leader = $(if ($LeaderKey) { $LeaderKey } else { 'config' })
                extra_config = @($ExtraConfig)
                foreground = $foreground; size = $size; marker = $marker; exe = $Exe
                config = $(if ($NoConfig) { '-n' } else { $ConfigFile })
                utc = (Get-Date).ToUniversalTime().ToString('o')
            }) $null
    }
}
finally {
    # 输入法状态可能是全局共享的：还原发键前的转换模式，别把用户的中/英文状态留在字母数字。
    if ($imeBefore -ge 0 -and $h -ne [IntPtr]::Zero) { [SmokeWin32]::ImeConversion($h, $imeBefore) | Out-Null }
    if ($proc) {
        & taskkill.exe /PID $proc.Id /T /F 2>&1 | Out-Null
        $proc.WaitForExit(5000) | Out-Null
    }
}
