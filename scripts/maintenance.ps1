param(
    [switch]$Apply,
    [switch]$Scheduled,
    [switch]$Deep
)

$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).ProviderPath.TrimEnd('\')
$stateDir = Join-Path $env:LOCALAPPDATA 'ZiDevTools\maintenance'
$statePath = Join-Path $stateDir 'last-run.json'
$now = Get-Date
$month = $now.ToString('yyyy-MM')
$lastDeepMonth = $null
if (Test-Path -LiteralPath $statePath) {
    try {
        $lastDeepMonth = (Get-Content -LiteralPath $statePath -Raw -ErrorAction Stop |
            ConvertFrom-Json -ErrorAction Stop).lastDeepMonth
    } catch {
        Write-Warning '上次维护记录无法读取，本轮将按尚未完成月度清理处理。'
    }
}
$fullCleanup = $Deep -or ($Scheduled -and $lastDeepMonth -ne $month)
$cleanupPaths = if ($fullCleanup) {
    @('target')
} else {
    @('target\debug\incremental', 'target\release\incremental')
}

function Save-MaintenanceRecord([string]$result, [long]$bytes) {
    New-Item -ItemType Directory -Path $stateDir -Force | Out-Null
    $record = [ordered]@{
        time = (Get-Date).ToString('o')
        result = $result
        mode = if ($fullCleanup) { 'deep' } else { 'incremental' }
        reclaimedBytes = $bytes
        lastDeepMonth = if ($result -eq 'completed' -and $fullCleanup) { $month } else { $lastDeepMonth }
    }
    $record | ConvertTo-Json | Set-Content -LiteralPath $statePath -Encoding utf8
}

function Assert-SafeTarget([string]$relative) {
    $absolute = [IO.Path]::GetFullPath((Join-Path $projectRoot $relative))
    $prefix = $projectRoot + [IO.Path]::DirectorySeparatorChar
    if (-not $absolute.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "清理目标超出项目目录：$relative"
    }
    if (-not (Test-Path -LiteralPath $absolute)) { return $null }
    $item = Get-Item -LiteralPath $absolute -Force
    if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "清理目标不是普通目录：$absolute"
    }
    $links = @(Get-ChildItem -LiteralPath $absolute -Recurse -Force -Attributes ReparsePoint -ErrorAction Stop)
    if ($links.Count -gt 0) { throw "清理目标包含链接，已停止：$absolute" }
    return $absolute
}

if ($Apply) {
    $busy = @(Get-CimInstance Win32_Process -Filter "Name = 'cargo.exe' OR Name = 'rustc.exe' OR Name = 'rustdoc.exe' OR Name = 'wix.exe'" -ErrorAction Stop)
    if ($busy.Count -gt 0) {
        $message = '检测到 Rust/WiX 构建进程，跳过本轮维护。'
        if ($Scheduled) {
            Save-MaintenanceRecord 'skipped-build-active' 0
            Write-Output $message
            exit 0
        }
        throw $message
    }
}

$reclaimed = [long]0
foreach ($relative in $cleanupPaths) {
    $absolute = Assert-SafeTarget $relative
    if (-not $absolute) {
        Write-Output "跳过（不存在）：$relative"
        continue
    }
    $bytes = [long](Get-ChildItem -LiteralPath $absolute -Recurse -Force -File -ErrorAction Stop |
        Measure-Object -Property Length -Sum).Sum
    Write-Output ('{0}: {1:N2} GiB' -f $relative, ($bytes / 1GB))
    if ($Apply) {
        Remove-Item -LiteralPath $absolute -Recurse -Force -ErrorAction Stop
        if (Test-Path -LiteralPath $absolute) { throw "清理后目录仍存在：$absolute" }
        $reclaimed += $bytes
    }
}

if ($Apply) {
    Save-MaintenanceRecord 'completed' $reclaimed
    Write-Output ('本轮已释放约 {0:N2} GiB；阶段归档、发布包、源码及知识索引未触及。' -f ($reclaimed / 1GB))
    Write-Output "维护记录：$statePath"
} else {
    Write-Output '预览模式。加 -Apply 才会清理上述构建缓存；-Deep 预览或清理整个 target。'
}
