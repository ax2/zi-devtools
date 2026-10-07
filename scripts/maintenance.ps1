param(
    [switch]$Apply,
    [switch]$Scheduled,
    [switch]$Deep,
    [switch]$BudgetOnly,
    [ValidateRange(0, 1024)][double]$BudgetGiB = 0,
    [string]$BuildCacheRoot
)
$ErrorActionPreference = 'Stop'
if ($Deep -and $BudgetOnly) { throw '-Deep and -BudgetOnly are mutually exclusive.' }
# Resolve the existing workspace volume mount before checking/deleting paths.
if (-not ('ZiStorage.NativePath' -as [type])) {
    Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
namespace ZiStorage {
 public static class NativePath {
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  static extern SafeFileHandle CreateFile(string p, uint a, uint s, IntPtr x, uint c, uint f, IntPtr t);
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  static extern uint GetFinalPathNameByHandle(SafeFileHandle h, StringBuilder b, uint n, uint f);
  public static string Get(string p) {
   using(var h=CreateFile(p,0,7,IntPtr.Zero,3,0x02000000,IntPtr.Zero)) {
    if(h.IsInvalid) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
    var b=new StringBuilder(32768);
    uint n=GetFinalPathNameByHandle(h,b,(uint)b.Capacity,0);
    if(n==0 || n>=b.Capacity) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
    string r=b.ToString();
    if(r.StartsWith(@"\\?\UNC\")) return @"\\"+r.Substring(8);
    return r.StartsWith(@"\\?\") ? r.Substring(4) : r;
   }
  }
 }
}
"@
}
$maintenanceLock = $null
if ($Apply) {
    $maintenanceLock = [Threading.Mutex]::new($false, 'Local\ZiDevToolsStorageMaintenance')
    try { $entered = $maintenanceLock.WaitOne(0) } catch [Threading.AbandonedMutexException] { $entered = $true }
    if (-not $entered) {
        $maintenanceLock.Dispose()
        if ($Scheduled) { Write-Output '已有维护运行，跳过。'; exit 0 }
        throw '已有维护运行，拒绝并发清理。'
    }
}
try {
$projectRoot = [ZiStorage.NativePath]::Get((Join-Path $PSScriptRoot '..')).TrimEnd('\')
$stateDir = Join-Path $env:LOCALAPPDATA 'ZiDevTools\maintenance'
$statePath = Join-Path $stateDir 'last-run.json'
$month = (Get-Date).ToString('yyyy-MM')
$lastDeepMonth = $null
if (Test-Path -LiteralPath $statePath) {
    $lastDeepMonth = (Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json).lastDeepMonth
}
function Assert-NoLinks([string]$path) {
    $cursor = Get-Item -LiteralPath $path -Force
    while ($null -ne $cursor) {
        if ($cursor.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "拒绝链接路径：$($cursor.FullName)" }
        $cursor = $cursor.Parent
    }
    if (@(Get-ChildItem -LiteralPath $path -Recurse -Force -Attributes ReparsePoint).Count) {
        throw "拒绝包含链接的目标：$path"
    }
}
function Safe-Target([string]$root, [string]$relative) {
    $path = [IO.Path]::GetFullPath((Join-Path $root $relative))
    if (-not $path.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase)) { throw "目标越界：$path" }
    if (-not (Test-Path -LiteralPath $path)) { return $null }
    if (-not (Get-Item -LiteralPath $path -Force).PSIsContainer) { throw "不是目录：$path" }
    Assert-NoLinks $path
    return $path
}
function Size-Bytes([string]$path) {
    if (-not $path) { return [long]0 }
    return [long](Get-ChildItem -LiteralPath $path -Recurse -Force -File | Measure-Object Length -Sum).Sum
}
Assert-NoLinks $projectRoot
$roots = @([pscustomobject]@{Root=$projectRoot; Relative='target'})
if ($BuildCacheRoot) {
    # Explicit external cache scope. Release builds and shared CARGO_HOME stay intact.
    $external = [IO.Path]::GetFullPath($BuildCacheRoot).TrimEnd('\')
    if ((Split-Path $external -Leaf) -ne 'zi-devtools') { throw '外置缓存目录必须名为 zi-devtools。' }
    if (-not (Test-Path -LiteralPath $external -PathType Container)) { throw "外置缓存不存在：$external" }
    Assert-NoLinks $external
    if ($external -eq $projectRoot -or $external.StartsWith($projectRoot+'\', [StringComparison]::OrdinalIgnoreCase)) {
        throw '外置缓存必须与项目目录分离。'
    }
    $roots += [pscustomobject]@{Root=$external; Relative='debug'}
}
# Legacy scheduled/budget invocations are always audit-only, even with -Apply/-Deep.
# Rebuildable does not mean disposable: clearing useful caches slows development.
$manualDeep = $Deep -and -not $Scheduled -and -not $BudgetOnly
$targets = @()
$usage = @()
foreach ($root in $roots) {
    $whole = Safe-Target $root.Root $root.Relative
    $bytes = Size-Bytes $whole
    $over = $BudgetGiB -gt 0 -and $bytes -gt ($BudgetGiB * 1GB)
    $usage += [pscustomobject]@{path=(Join-Path $root.Root $root.Relative); bytes=$bytes; overBudget=$over}
    Write-Output ('{0}: {1:N3} GiB；保留缓存，容量不触发删除。' -f $usage[-1].path, ($bytes/1GB))
    if ($manualDeep) {
        # Never remove the whole project target: release/WASI outputs remain intact.
        $part = if ($root.Relative -eq 'target') { 'target\debug' } else { 'debug' }
        $candidate = Safe-Target $root.Root $part
        if ($candidate) { $targets += $candidate }
    }
}
function Assert-Idle {
    $busy = @(Get-CimInstance Win32_Process | Where-Object {
        if ($_.Name -match '^(cargo|rustc|rustdoc|wix|light|candle)\.exe$') { return $true }
        if ($_.ExecutablePath) {
            $exe = [ZiStorage.NativePath]::Get($_.ExecutablePath)
            foreach ($root in $roots) {
                $prefix = (Join-Path $root.Root $root.Relative) + '\'
                if ($exe.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) { return $true }
            }
        }
        return $false
    })
    if ($busy.Count) { throw '检测到构建进程或缓存中的运行程序，跳过维护。' }
}
$reclaimed = [long]0
$result = if ($manualDeep) { 'completed' } else { 'retained' }
if ($Scheduled -or ($Apply -and $targets.Count)) {
    try { Assert-Idle } catch {
        if (-not $Scheduled) { throw }
        $result='skipped-build-active'
        $targets=@()
    }
}
foreach ($target in $targets) {
    Write-Output "候选清理：$target"
    if ($Apply) {
        Assert-Idle
        Assert-NoLinks $target
        $bytes = Size-Bytes $target
        Remove-Item -LiteralPath $target -Recurse -Force
        if (Test-Path -LiteralPath $target) { throw "清理后仍存在：$target" }
        $reclaimed += $bytes
    }
}
if ($Apply) {
    New-Item -ItemType Directory -Path $stateDir -Force | Out-Null
    $record = [ordered]@{
        time=(Get-Date).ToString('o'); result=$result; reclaimedBytes=$reclaimed
        mode=if($manualDeep){'manual-debug-cleanup'}else{'audit'}
        budgetGiB=$BudgetGiB; usageBefore=$usage
        lastDeepMonth=if($result -eq 'completed' -and $manualDeep){$month}else{$lastDeepMonth}
    }
        $record | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $statePath -Encoding utf8
    $record | ConvertTo-Json -Depth 5 -Compress | Add-Content -LiteralPath (Join-Path $stateDir 'history.jsonl') -Encoding utf8
    Write-Output ('释放 {0:N3} GiB；记录：{1}' -f ($reclaimed/1GB), $statePath)
} else { Write-Output '仅预览。默认保留缓存；只有手动 -Deep -Apply 才会删除 debug 缓存，定时任务始终不删除。' }

} finally {
    if ($maintenanceLock) { $maintenanceLock.ReleaseMutex(); $maintenanceLock.Dispose() }
}
