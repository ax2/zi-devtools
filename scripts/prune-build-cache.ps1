param([Parameter(Mandatory=$true)][string]$Plan, [switch]$Apply)
$ErrorActionPreference='Stop'
$projectRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..')).TrimEnd('\')
$targetRoot=Join-Path $projectRoot 'target'
$document=Get-Content -LiteralPath $Plan -Raw | ConvertFrom-Json
if($document.schemaVersion -ne 1 -or $document.projectRoot -ne $projectRoot){throw 'Plan belongs to another workspace or schema.'}
function Assert-NoLinks([string]$path) {
    $cursor=Get-Item -LiteralPath $path -Force
    while($cursor){
        if($cursor.Attributes -band [IO.FileAttributes]::ReparsePoint){throw "Linked path rejected: $path"}
        $cursor=$cursor.Parent
    }
    if((Get-Item -LiteralPath $path).PSIsContainer -and @(Get-ChildItem -LiteralPath $path -Recurse -Force -Attributes ReparsePoint).Count){throw "Linked child rejected: $path"}
}
function Assert-Idle {
    $busy=@(Get-CimInstance Win32_Process | Where-Object {
        if($_.ExecutablePath -and $_.ExecutablePath.StartsWith($targetRoot+'\',[StringComparison]::OrdinalIgnoreCase)){return $true}
        if($_.Name -notmatch '^(cargo|rustc|rustdoc|wix|light|candle)\.exe$'){return $false}
        $command=([string]$_.CommandLine).Replace('/','\')
        # Only exempt Cargo with an explicit absolute manifest in another project.
        # An ambiguous build still blocks deletion; no process is interrupted.
        if($_.Name -eq 'cargo.exe' -and $command -notlike "*$projectRoot\*"){
            $match=[regex]::Match($_.CommandLine,'--manifest-path\s+(?:"([^"]+Cargo\.toml)"|([^\s]+Cargo\.toml))',[Text.RegularExpressions.RegexOptions]::IgnoreCase)
            if($match.Success){
                $manifest=if($match.Groups[1].Success){$match.Groups[1].Value}else{$match.Groups[2].Value}
                if([IO.Path]::IsPathFullyQualified($manifest) -and -not [IO.Path]::GetFullPath($manifest).StartsWith($projectRoot+'\',[StringComparison]::OrdinalIgnoreCase)){return $false}
            }
        }
        if($_.Name -in @('rustc.exe','rustdoc.exe') -and $command -notlike "*$projectRoot\*"){
            $output=[regex]::Match($_.CommandLine,'--out-dir\s+(?:"([^"]+)"|([^\s]+))')
            if($output.Success){
                $directory=if($output.Groups[1].Success){$output.Groups[1].Value}else{$output.Groups[2].Value}
                if([IO.Path]::IsPathFullyQualified($directory) -and -not [IO.Path]::GetFullPath($directory).StartsWith($projectRoot+'\',[StringComparison]::OrdinalIgnoreCase)){return $false}
            }
        }
        return $true
    })
    if($busy.Count){throw 'Build or target executable is active; no cleanup.'}
}
function Measure-Target([string]$path) {
    $item=Get-Item -LiteralPath $path -Force
    $files=if($item.PSIsContainer){@(Get-ChildItem -LiteralPath $path -Recurse -Force -File)}else{@($item)}
    $sum=($files | Measure-Object Length -Sum).Sum
    $latest=$item.LastWriteTimeUtc.Ticks
    foreach($file in $files){$latest=[Math]::Max($latest,$file.LastWriteTimeUtc.Ticks)}
    return @{bytes=[long]$sum;files=$files.Count;latestTicks=$latest}
}
$mutex=[Threading.Mutex]::new($false,'Local\ZiDevToolsBuild')
$entered=$false
try {
    try{$entered=$mutex.WaitOne(0)}catch [Threading.AbandonedMutexException]{$entered=$true}
    if(-not $entered){throw 'Another build/cleanup owns the workspace lock.'}
    Assert-NoLinks $projectRoot
    Assert-Idle
    $validated=@();$seen=[Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach($row in $document.targets){
        $relative=[string]$row.relative
        $path=[IO.Path]::GetFullPath((Join-Path $projectRoot $relative))
        if(-not $path.StartsWith($targetRoot+'\',[StringComparison]::OrdinalIgnoreCase) -or -not $seen.Add($path)){throw "Invalid/duplicate target: $relative"}
        $allowedDir=$relative -match '^target[\\/]debug[\\/](incremental|\.fingerprint)[\\/][A-Za-z0-9_-]+$' -or $relative -in @('target/plugin-size','target/wasm32-wasip1/plugin-size')
        $allowedFile=$relative -match '^target[\\/]debug[\\/](deps|examples)[\\/][A-Za-z0-9_.-]+$'
        $item=Get-Item -LiteralPath $path -Force
        if(($row.kind -eq 'directory' -and (-not $allowedDir -or -not $item.PSIsContainer)) -or ($row.kind -eq 'file' -and (-not $allowedFile -or $item.PSIsContainer)) -or $row.kind -notin @('file','directory')){throw "Target type/scope rejected: $relative"}
        Assert-NoLinks $path
        $measure=Measure-Target $path
        if($measure.bytes -ne $row.bytes -or $measure.files -ne $row.files -or $measure.latestTicks -ne $row.latestTicks){throw "Target changed since preview: $relative"}
        $validated+=@{path=$path;row=$row}
    }
    $total=[long](($validated.row | Measure-Object bytes -Sum).Sum)
    Write-Output ('Validated {0} targets, {1:N3} GiB. {2}' -f $validated.Count,($total/1GB),$(if($Apply){'Applying approved plan.'}else{'Preview only.'}))
    $released=[long]0;$removed=0;$timer=[Diagnostics.Stopwatch]::StartNew()
    if($Apply){
        Assert-Idle
        foreach($entry in $validated){
            if($timer.Elapsed.TotalSeconds -ge 10){Assert-Idle;$timer.Restart()}
            Assert-NoLinks $entry.path
            $measure=Measure-Target $entry.path
            if($measure.bytes -ne $entry.row.bytes -or $measure.files -ne $entry.row.files -or $measure.latestTicks -ne $entry.row.latestTicks){throw 'Target changed during cleanup.'}
            if($entry.row.kind -eq 'directory'){Remove-Item -LiteralPath $entry.path -Recurse -Force}else{Remove-Item -LiteralPath $entry.path -Force}
            if(Test-Path -LiteralPath $entry.path){throw 'Target still exists after cleanup.'}
            $released+=$entry.row.bytes;$removed++
            if($removed % 200 -eq 0){Write-Output ('Removed {0}/{1}; {2:N3} GiB logical bytes.' -f $removed,$validated.Count,($released/1GB))}
        }
    }
    $result=@{time=(Get-Date).ToString('o');applied=[bool]$Apply;targets=$validated.Count;removed=$removed;reclaimedLogicalBytes=$released;projectRoot=$projectRoot}
    $result | ConvertTo-Json | Set-Content -LiteralPath ($Plan+$(if($Apply){'.applied.json'}else{'.preview.json'})) -Encoding utf8
    $result | ConvertTo-Json
} finally {if($entered){$mutex.ReleaseMutex()};$mutex.Dispose()}
