<#
.SYNOPSIS
  Windows 性能探针：在一个场景下跑 wezterm-gui，采集 periodic_stat_logging 指标、CPU 与 GPU 占用并汇总。

.DESCRIPTION
  启动 `wezterm-gui.exe [--config-file ..] --config periodic_stat_logging=10 [--config max_fps=<n>]
  start --always-new-process -- pwsh ... <场景命令>`，stderr 重定向到 metrics-<scenario>-<fps>.txt；
  -Seconds 秒内每秒采样：
    * CPU：wezterm-gui 进程（及其子进程树：OpenConsole/pwsh）的 TotalProcessorTime 增量，
      除以逻辑核数归一化，写 cpu-<scenario>.csv。按 PID 取数而不是 Get-Counter 的
      \Process(wezterm-gui*) 实例名，避免本机还开着别的 wezterm-gui 时实例名（wezterm-gui#1）串号；
    * GPU：`\GPU Engine(pid_<pid>_*)\Utilization Percentage`（任意厂商，按进程）写 gpu-engine-<scenario>.csv；
      有 nvidia-smi 时另跑 `nvidia-smi pmon -s u -d 1 -c <Seconds>` 写 gpu-<scenario>.txt
      （Windows WDDM 下 pmon 常全是 "-"，此时汇总里记 unavailable）。
  结束后解析 metrics 文件，写 summary-<scenario>.json。同名输出已存在时拒绝覆盖（换批次目录）。

  场景（命令都在窗格里跑，时长比 -Seconds 多留 30 秒余量）：
    idle     Start-Sleep
    cat      Get-Content -Raw 读入把 Cargo.lock 重复 20 次拼成的临时文件（放仓内 .local/tmp，
             结束后删除），循环整段输出到终端
    loop     循环 1..200000 | % { $_ }
    scroll   先输出 60000 行，再用 SendInput 连发 Shift+PageUp/PageDown 来回滚动；
             发键前必须确认前台窗口属于被测 wezterm，否则只测输出阶段并在汇总里标注
    spinner  10Hz 刷新窗口标题（OSC 0，盲文转圈帧）

  这是真实桌面窗口：运行期间窗口会出现在桌面上，请不要同时操作本机键鼠，否则会污染数据。

.PARAMETER Out
  批次目录（必填）。相对路径按仓库根解析。
.PARAMETER Exe
  wezterm-gui.exe 路径，默认 <仓库>/target/release/wezterm-gui.exe。
.PARAMETER ConfigFile
  wezterm.lua，默认 <仓库>/dotfiles/wezterm-config/wezterm.lua；-NoConfig 用 -n 基线配置。
.PARAMETER Scenario
  idle | cat | loop | scroll | spinner
.PARAMETER Seconds
  采样时长（默认 60）。
.PARAMETER MaxFps
  数字：附加 --config max_fps=<n>；follow（默认）：不覆盖，沿用配置里的值。
.PARAMETER ExtraConfig
  额外的 --config 覆盖（每项形如 key=value，Lua 表达式）；排查用，会记入汇总的 config_overrides。
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$Exe,
    [string]$ConfigFile,
    [switch]$NoConfig,
    [Parameter(Mandatory = $true)][ValidateSet('idle', 'cat', 'loop', 'scroll', 'spinner')][string]$Scenario,
    [ValidateRange(10, 3600)][int]$Seconds = 60,
    [ValidatePattern('^(follow|\d{1,4})$')][string]$MaxFps = 'follow',
    [string[]]$ExtraConfig = @()
)

$ErrorActionPreference = 'Stop'
$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path

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

$MetricsFile = Join-Path $OutDir "metrics-$Scenario-$MaxFps.txt"
$CpuFile = Join-Path $OutDir "cpu-$Scenario.csv"
$GpuFile = Join-Path $OutDir "gpu-$Scenario.txt"
$GpuEngineFile = Join-Path $OutDir "gpu-engine-$Scenario.csv"
$SummaryFile = Join-Path $OutDir "summary-$Scenario.json"
foreach ($f in @($MetricsFile, $CpuFile, $GpuFile, $GpuEngineFile, $SummaryFile)) {
    if (Test-Path -LiteralPath $f) { throw "$f 已存在，拒绝覆盖；换一个批次目录" }
}

if (-not ('PerfWin32' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class PerfWin32 {
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
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern uint MapVirtualKey(uint code, uint type);
    [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] inputs, int size);

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

    public static uint ForegroundPid() {
        uint p = 0;
        IntPtr h = GetForegroundWindow();
        if (h != IntPtr.Zero) { GetWindowThreadProcessId(h, out p); }
        return p;
    }

    // 单个键盘事件；extended 用于 PageUp/PageDown 等导航键（lParam 的扩展键位）。
    public static void Key(ushort vk, bool up, bool extended) {
        INPUT[] a = new INPUT[1];
        a[0].type = 1;
        a[0].u.ki.wVk = vk;
        a[0].u.ki.wScan = (ushort)MapVirtualKey(vk, 0);
        a[0].u.ki.dwFlags = (up ? 2u : 0u) | (extended ? 1u : 0u);
        if (SendInput(1, a, Marshal.SizeOf(typeof(INPUT))) != 1) { throw new System.ComponentModel.Win32Exception(); }
    }
}
'@
}
[PerfWin32]::SetProcessDPIAware() | Out-Null

function Quote-Arg([string]$s) {
    if ($s -match '[\s"]') { return '"' + ($s -replace '"', '\"') + '"' }
    return $s
}

# ---------- 场景命令 ----------
$paneSeconds = $Seconds + 30
$tempInput = $null
$scenarioBody = switch ($Scenario) {
    'idle' { "Start-Sleep $paneSeconds" }
    'cat' {
        $tempDir = Join-Path $Repo '.local/tmp'
        New-Item -ItemType Directory -Force -Path $tempDir | Out-Null
        $tempInput = Join-Path $tempDir "perf-cat-$PID.txt"
        $lock = Get-Content -Raw -LiteralPath (Join-Path $Repo 'Cargo.lock')
        [IO.File]::WriteAllText($tempInput, ($lock * 20), (New-Object Text.UTF8Encoding $false))
        @"
`$text = Get-Content -Raw -LiteralPath '$tempInput'
`$end = (Get-Date).AddSeconds($paneSeconds)
while ((Get-Date) -lt `$end) { [Console]::Out.Write(`$text) }
"@
    }
    'loop' {
        @"
`$end = (Get-Date).AddSeconds($paneSeconds)
while ((Get-Date) -lt `$end) { 1..200000 | % { `$_ } }
"@
    }
    'scroll' {
        @"
1..60000 | % { "scroll-line `$_ " + ('x' * 60) }
Start-Sleep $paneSeconds
"@
    }
    'spinner' {
        @"
`$e = [char]27; `$bel = [char]7
`$frames = [char[]]'⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏'
`$end = (Get-Date).AddSeconds($paneSeconds)
`$i = 0
while ((Get-Date) -lt `$end) {
    [Console]::Out.Write("`$e]0;" + `$frames[`$i % `$frames.Length] + " work`$bel")
    `$i++
    Start-Sleep -Milliseconds 100
}
"@
    }
}
$stamp = (Get-Date).ToUniversalTime().ToString('yyyyMMddHHmmss')
$inner = "Write-Host 'WEZTERM-PERF-PROBE-$Scenario-$stamp'`n$scenarioBody"
$shell = (Get-Command pwsh -ErrorAction SilentlyContinue).Source
if (-not $shell) { $shell = (Get-Command powershell -ErrorAction Stop).Source }
$encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($inner))

$parts = @()
if ($NoConfig) { $parts += '-n' } else { $parts += '--config-file'; $parts += (Quote-Arg $ConfigFile) }
$parts += @('--config', 'periodic_stat_logging=10')
if ($MaxFps -ne 'follow') { $parts += @('--config', "max_fps=$MaxFps") }
foreach ($kv in $ExtraConfig) { $parts += @('--config', (Quote-Arg $kv)) }
$parts += @('start', '--always-new-process', '--', (Quote-Arg $shell), '-NoLogo', '-NoProfile', '-EncodedCommand', $encoded)
$argString = $parts -join ' '

# ---------- 采样辅助 ----------
$cores = [Environment]::ProcessorCount

function Get-Descendants([int]$RootPid) {
    $all = Get-CimInstance -ClassName Win32_Process -Property ProcessId, ParentProcessId
    $kids = @{}
    foreach ($p in $all) {
        if (-not $kids.ContainsKey([int]$p.ParentProcessId)) { $kids[[int]$p.ParentProcessId] = @() }
        $kids[[int]$p.ParentProcessId] += [int]$p.ProcessId
    }
    $result = New-Object System.Collections.Generic.List[int]
    $queue = New-Object System.Collections.Generic.Queue[int]
    $queue.Enqueue($RootPid)
    while ($queue.Count -gt 0) {
        $cur = $queue.Dequeue()
        if ($kids.ContainsKey($cur)) {
            foreach ($k in $kids[$cur]) { if (-not $result.Contains($k)) { $result.Add($k); $queue.Enqueue($k) } }
        }
    }
    return , $result.ToArray()
}

function Get-CpuSeconds([int]$ProcId) {
    try { return [Diagnostics.Process]::GetProcessById($ProcId).TotalProcessorTime.TotalSeconds } catch { return $null }
}

# ---------- 运行 ----------
$proc = $null
$pmon = $null
$h = [IntPtr]::Zero
$cpuRows = New-Object System.Collections.Generic.List[object]
$gpuRows = New-Object System.Collections.Generic.List[object]
$scrollNote = $null
$startUtc = (Get-Date).ToUniversalTime()
try {
    Write-Host "启动 $Exe（scenario=$Scenario seconds=$Seconds max_fps=$MaxFps，输出 $OutDir）"
    $proc = Start-Process -FilePath $Exe -ArgumentList $argString -WorkingDirectory $Repo -PassThru -RedirectStandardError $MetricsFile
    $deadline = (Get-Date).AddSeconds(20)
    while ((Get-Date) -lt $deadline -and $h -eq [IntPtr]::Zero) {
        if ($proc.HasExited) { throw "wezterm-gui 提前退出（exit=$($proc.ExitCode)），见 $MetricsFile" }
        $h = [PerfWin32]::FindWindow([uint32]$proc.Id)
        if ($h -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 200 }
    }
    if ($h -eq [IntPtr]::Zero) { throw '20 秒内没有出现 wezterm 主窗口' }

    if (Get-Command nvidia-smi -ErrorAction SilentlyContinue) {
        $pmon = Start-Process -FilePath (Get-Command nvidia-smi).Source -ArgumentList "pmon -s u -d 1 -c $Seconds" `
            -RedirectStandardOutput $GpuFile -WindowStyle Hidden -PassThru
    }
    else {
        Set-Content -LiteralPath $GpuFile -Value 'nvidia-smi 不可用（未在 PATH 中找到）' -Encoding utf8
    }

    $descendants = @()
    $prev = @{}
    $prevTime = [Diagnostics.Stopwatch]::StartNew()
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $guiPid = $proc.Id
    $prev[$guiPid] = Get-CpuSeconds $guiPid
    $lastTick = 0.0
    $scrollSent = 0
    $scrollDir = $false   # $false = PageUp
    $i = 0
    # 按墙钟计时：单次迭代偶尔超过 1 秒（Get-CimInstance、发键）时不拉长总时长。
    while ($clock.Elapsed.TotalSeconds -lt $Seconds) {
        $i++
        $target = $i * 1000
        $elapsedSec = [int]$clock.Elapsed.TotalSeconds
        # 滚动输入：从第 8 秒起（等 60000 行输出结束）到最后。
        if ($Scenario -eq 'scroll' -and $elapsedSec -ge 8) {
            if ([PerfWin32]::ForegroundPid() -eq $guiPid) {
                for ($k = 0; $k -lt 12; $k++) {
                    $vk = if ($scrollDir) { 0x22 } else { 0x21 }   # VK_NEXT / VK_PRIOR
                    [PerfWin32]::Key(0xA0, $false, $false)
                    [PerfWin32]::Key([uint16]$vk, $false, $true)
                    [PerfWin32]::Key([uint16]$vk, $true, $true)
                    [PerfWin32]::Key(0xA0, $true, $false)
                    $scrollSent++
                    Start-Sleep -Milliseconds 40
                }
                if ($i % 5 -eq 0) { $scrollDir = -not $scrollDir }
            }
            elseif (-not $scrollNote) {
                $scrollNote = "第 $elapsedSec 秒起前台窗口不属于被测 wezterm，已停止发送滚动键（只测到输出阶段与此前的滚动）"
            }
        }

        if (($i % 5) -eq 1) { $descendants = Get-Descendants $guiPid }
        $now = $prevTime.Elapsed.TotalSeconds
        $dt = $now - $lastTick
        $lastTick = $now
        $guiCpu = Get-CpuSeconds $guiPid
        $guiDelta = if ($null -ne $guiCpu -and $null -ne $prev[$guiPid]) { [Math]::Max(0.0, $guiCpu - $prev[$guiPid]) } else { 0.0 }
        if ($null -ne $guiCpu) { $prev[$guiPid] = $guiCpu }
        $treeDelta = $guiDelta
        foreach ($c in $descendants) {
            $cs = Get-CpuSeconds $c
            if ($null -eq $cs) { continue }
            # 新出现的进程只记基线、不计其启动期 CPU：汇总要的是稳态占用。
            if ($prev.ContainsKey($c)) { $treeDelta += [Math]::Max(0.0, $cs - $prev[$c]) }
            $prev[$c] = $cs
        }
        if ($dt -gt 0) {
            $cpuRows.Add([pscustomobject]@{
                    t = [Math]::Round($clock.Elapsed.TotalSeconds, 2)
                    gui_norm_pct = [Math]::Round($guiDelta / $dt / $cores * 100, 3)
                    gui_core_pct = [Math]::Round($guiDelta / $dt * 100, 2)
                    tree_norm_pct = [Math]::Round($treeDelta / $dt / $cores * 100, 3)
                    tree_core_pct = [Math]::Round($treeDelta / $dt * 100, 2)
                })
        }

        # GPU Engine 计数器：按 PID 累加各引擎利用率（引擎实例随使用动态出现，每次用通配取）。
        $g3d = 0.0; $gall = 0.0
        try {
            $samples = (Get-Counter -Counter "\GPU Engine(pid_${guiPid}_*)\Utilization Percentage" -ErrorAction Stop).CounterSamples
            foreach ($s in $samples) {
                $gall += $s.CookedValue
                if ($s.InstanceName -match 'engtype_3D$') { $g3d += $s.CookedValue }
            }
        }
        catch { }
        $gpuRows.Add([pscustomobject]@{ t = [Math]::Round($clock.Elapsed.TotalSeconds, 2); gpu_3d_pct = [Math]::Round($g3d, 3); gpu_all_pct = [Math]::Round($gall, 3) })

        $remaining = $target - $clock.ElapsedMilliseconds
        if ($remaining -gt 0) { Start-Sleep -Milliseconds ([int]$remaining) }
    }
    # 再等一小会儿，让最后一次 10 秒快照落盘。
    Start-Sleep -Seconds 2
}
finally {
    if ($pmon -and -not $pmon.HasExited) { Stop-Process -Id $pmon.Id -Force -ErrorAction SilentlyContinue }
    if ($proc) {
        & taskkill.exe /PID $proc.Id /T /F 2>&1 | Out-Null
        $proc.WaitForExit(5000) | Out-Null
    }
    if ($tempInput -and (Test-Path -LiteralPath $tempInput)) { Remove-Item -LiteralPath $tempInput -Force -ErrorAction SilentlyContinue }
}

$cpuRows | Export-Csv -LiteralPath $CpuFile -NoTypeInformation -Encoding utf8
$gpuRows | Export-Csv -LiteralPath $GpuEngineFile -NoTypeInformation -Encoding utf8

# ---------- 解析 metrics ----------
function ConvertTo-Ms([string]$Text) {
    if ($Text -notmatch '^([\d.]+)(ns|µs|us|ms|s)$') { return $null }
    $v = [double]::Parse($Matches[1], [Globalization.CultureInfo]::InvariantCulture)
    switch ($Matches[2]) {
        'ns' { return $v / 1e6 }
        'µs' { return $v / 1e3 }
        'us' { return $v / 1e3 }
        'ms' { return $v }
        's' { return $v * 1e3 }
    }
}

# 每个 rate 表是一次快照：键 -> @{ current; p50; p75; p95 }；latency 表是该快照的直方图分位。
$snapshots = New-Object System.Collections.Generic.List[hashtable]
$mode = $null
$metricLines = if (Test-Path -LiteralPath $MetricsFile) { Get-Content -LiteralPath $MetricsFile -Encoding UTF8 } else { @() }
foreach ($line in $metricLines) {
    if ($line -match '^STAT\s+current\s+p50\s+p75\s+p95') {
        $snapshots.Add(@{ rate = @{}; latency = @{} }); $mode = 'rate'; continue
    }
    if ($line -match '^STAT\s+p50\s+p75\s+p95') { $mode = 'latency'; continue }
    if ($line -match '^STAT\s+COUNT') { $mode = $null; continue }
    if ($snapshots.Count -eq 0 -or -not $mode) { continue }
    if ($mode -eq 'rate' -and $line -match '^Key\((?<k>[^)]+)\)\s+(?<c>\d+(\.\d+)?)\s+(?<a>\d+(\.\d+)?)\s+(?<b>\d+(\.\d+)?)\s+(?<d>\d+(\.\d+)?)\s*$') {
        $snapshots[$snapshots.Count - 1].rate[$Matches.k] = @{ current = [double]$Matches.c; p50 = [double]$Matches.a; p75 = [double]$Matches.b; p95 = [double]$Matches.d }
    }
    elseif ($mode -eq 'latency' -and $line -match '^Key\((?<k>[^)]+)\)\s+(?<a>\S+)\s+(?<b>\S+)\s+(?<d>\S+)\s*$') {
        $snapshots[$snapshots.Count - 1].latency[$Matches.k] = @{ p50 = ConvertTo-Ms $Matches.a; p75 = ConvertTo-Ms $Matches.b; p95 = ConvertTo-Ms $Matches.d }
    }
}

# 稳态：快照 >= 3 个时丢掉第一个（含启动瞬态）。
$steady = if ($snapshots.Count -ge 3) { @($snapshots | Select-Object -Skip 1) } else { @($snapshots) }

function Get-RateStat([string]$Key) {
    if ($snapshots.Count -eq 0) { return $null }
    $last = $snapshots[$snapshots.Count - 1].rate[$Key]
    $currents = @($steady | Where-Object { $_.rate.ContainsKey($Key) } | ForEach-Object { $_.rate[$Key].current })
    $p95s = @($snapshots | Where-Object { $_.rate.ContainsKey($Key) } | ForEach-Object { $_.rate[$Key].p95 })
    if (-not $last) { return $null }
    return [ordered]@{
        current_last = $last.current
        current_mean = if ($currents.Count) { [Math]::Round(($currents | Measure-Object -Average).Average, 2) } else { $null }
        current_max  = if ($currents.Count) { ($currents | Measure-Object -Maximum).Maximum } else { $null }
        p50_last     = $last.p50
        p95_last     = $last.p95
        p95_max      = if ($p95s.Count) { ($p95s | Measure-Object -Maximum).Maximum } else { $null }
    }
}

function Get-CacheStat([string]$Name) {
    $hit = Get-RateStat "$Name.hit.rate"
    $miss = Get-RateStat "$Name.miss.rate"
    # 命中率三种口径：p50 / p95 是最后一个快照里 hit、miss 各自分位的比值（p95 看高负载秒），
    # current 是稳态快照里 hit/miss 当前窗口计数之和的比值；没出现过 miss 事件按 1.0。
    $ratios = [ordered]@{ p50 = $null; p95 = $null; current = $null }
    if ($hit) {
        $missP50 = if ($miss) { $miss.p50_last } else { 0.0 }
        $missP95 = if ($miss) { $miss.p95_last } else { 0.0 }
        $missSum = 0.0; $hitSum = 0.0
        foreach ($snap in $steady) {
            if ($snap.rate.ContainsKey("$Name.hit.rate")) { $hitSum += $snap.rate["$Name.hit.rate"].current }
            if ($snap.rate.ContainsKey("$Name.miss.rate")) { $missSum += $snap.rate["$Name.miss.rate"].current }
        }
        if (($hit.p50_last + $missP50) -gt 0) { $ratios.p50 = [Math]::Round($hit.p50_last / ($hit.p50_last + $missP50), 4) }
        if (($hit.p95_last + $missP95) -gt 0) { $ratios.p95 = [Math]::Round($hit.p95_last / ($hit.p95_last + $missP95), 4) }
        if (($hitSum + $missSum) -gt 0) { $ratios.current = [Math]::Round($hitSum / ($hitSum + $missSum), 4) }
    }
    return [ordered]@{ hit = $hit; miss = $miss; hit_ratio = $ratios }
}

$paintLatency = $null
if ($snapshots.Count -gt 0) { $paintLatency = $snapshots[$snapshots.Count - 1].latency['gui.paint.impl'] }

function Get-Mean($Rows, [string]$Field) {
    if (-not $Rows -or $Rows.Count -eq 0) { return $null }
    return [Math]::Round(($Rows | Measure-Object -Property $Field -Average).Average, 3)
}
function Get-Max($Rows, [string]$Field) {
    if (-not $Rows -or $Rows.Count -eq 0) { return $null }
    return [Math]::Round(($Rows | Measure-Object -Property $Field -Maximum).Maximum, 3)
}

# nvidia-smi pmon：只取被测进程树里有数值的行（WDDM 下多为 "-"）。
$pmonSummary = [ordered]@{ available = $false }
if (Test-Path -LiteralPath $GpuFile) {
    $treeIds = @($guiPid) + @($descendants)
    $sm = @()
    foreach ($l in (Get-Content -LiteralPath $GpuFile)) {
        if ($l -match '^\s*\d+\s+(?<pid>\d+)\s+\S+\s+(?<sm>\S+)\s+') {
            if ($treeIds -contains [int]$Matches.pid -and $Matches.sm -match '^\d+$') { $sm += [double]$Matches.sm }
        }
    }
    if ($sm.Count -gt 0) {
        $pmonSummary = [ordered]@{ available = $true; sm_mean = [Math]::Round(($sm | Measure-Object -Average).Average, 2); sm_max = ($sm | Measure-Object -Maximum).Maximum }
    }
    else {
        $pmonSummary = [ordered]@{ available = $false; reason = 'pmon 没有该进程的数值行（WDDM 常见）或 nvidia-smi 不可用；以 GPU Engine 计数器为准' }
    }
}

$git = { param($a) try { (& git -C $Repo @a 2>$null | Select-Object -First 1) } catch { $null } }
$summary = [ordered]@{
    scenario        = $Scenario
    seconds         = $Seconds
    max_fps         = $MaxFps
    exe             = $Exe
    config          = $(if ($NoConfig) { '-n' } else { $ConfigFile })
    commit          = & $git @('rev-parse', '--short', 'HEAD')
    started_utc     = $startUtc.ToString('o')
    config_overrides = @($ExtraConfig)
    logical_cores   = $cores
    snapshots       = $snapshots.Count
    note            = '指标分位来自 wezterm 的 per-second 直方图（只含有事件的秒）；*_mean 取稳态快照（>=3 个快照时去掉第一个）；CPU 百分比 norm=除以逻辑核数，core=占单核百分比'
    paint           = [ordered]@{
        rate        = Get-RateStat 'gui.paint.impl.rate'
        latency_ms  = $paintLatency
    }
    caches          = [ordered]@{
        shape_cache             = Get-CacheStat 'shape_cache'
        line_quad_cache         = Get-CacheStat 'line_quad_cache'
        line_to_ele_shape_cache = Get-CacheStat 'line_to_ele_shape_cache'
        glyph_cache             = Get-CacheStat 'glyph_cache.glyph_cache'
        image_cache             = Get-CacheStat 'glyph_cache.image_cache'
    }
    atlas           = [ordered]@{
        allocate_failure_rate = Get-RateStat 'window.atlas.allocate.failure.rate'
        allocate_success_rate = Get-RateStat 'window.atlas.allocate.success.rate'
    }
    cpu             = [ordered]@{
        samples            = $cpuRows.Count
        gui_norm_pct_mean  = Get-Mean $cpuRows 'gui_norm_pct'
        gui_norm_pct_max   = Get-Max $cpuRows 'gui_norm_pct'
        gui_core_pct_mean  = Get-Mean $cpuRows 'gui_core_pct'
        tree_norm_pct_mean = Get-Mean $cpuRows 'tree_norm_pct'
        tree_norm_pct_max  = Get-Max $cpuRows 'tree_norm_pct'
        tree_core_pct_mean = Get-Mean $cpuRows 'tree_core_pct'
    }
    gpu             = [ordered]@{
        engine_3d_pct_mean  = Get-Mean $gpuRows 'gpu_3d_pct'
        engine_3d_pct_max   = Get-Max $gpuRows 'gpu_3d_pct'
        engine_all_pct_mean = Get-Mean $gpuRows 'gpu_all_pct'
        nvidia_smi_pmon     = $pmonSummary
    }
    scroll_input    = if ($Scenario -eq 'scroll') { [ordered]@{ chords_sent = $scrollSent; note = $scrollNote } } else { $null }
}
[IO.File]::WriteAllText($SummaryFile, (($summary | ConvertTo-Json -Depth 8) + "`n"), (New-Object Text.UTF8Encoding $false))
Write-Host "SUMMARY $SummaryFile"
Write-Host ("paint rate current_last={0} p95_last={1} | cpu gui norm mean={2}% tree norm mean={3}% | snapshots={4}" -f `
        $summary.paint.rate.current_last, $summary.paint.rate.p95_last, $summary.cpu.gui_norm_pct_mean, $summary.cpu.tree_norm_pct_mean, $snapshots.Count)

# ---------- 登记到 result.json（若批次目录已由 make evidence 分配） ----------
$resultPath = Join-Path $OutDir 'result.json'
if (Test-Path -LiteralPath $resultPath) {
    $r = Get-Content -Raw -Encoding UTF8 -LiteralPath $resultPath | ConvertFrom-Json
    $probe = [pscustomobject]@{ scenario = $Scenario; max_fps = $MaxFps; seconds = $Seconds; summary = "summary-$Scenario.json"; utc = (Get-Date).ToUniversalTime().ToString('o') }
    $probes = @(); if ($r.PSObject.Properties['probes']) { $probes = @($r.probes) }
    $probes = @($probes | Where-Object { $_.scenario -ne $Scenario }) + $probe
    $r | Add-Member -NotePropertyName probes -NotePropertyValue $probes -Force
    if ($r.status -eq 'PENDING') { $r.status = 'CAPTURED' }
    [IO.File]::WriteAllText($resultPath, (($r | ConvertTo-Json -Depth 8) + "`n"), (New-Object Text.UTF8Encoding $false))
}
