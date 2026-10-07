$ErrorActionPreference='Stop'
$base=[IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')
$fixture=Join-Path $base ('zi-storage-test-'+[guid]::NewGuid().ToString('N'))
$project=Join-Path $fixture 'project'
$external=Join-Path $fixture 'external\zi-devtools'
$oldLocal=$env:LOCALAPPDATA
$ownedProcess=$null
$junction=$null
function Check([bool]$ok,[string]$name){if(-not $ok){throw "FAIL $name"};Write-Output "PASS $name"}
try {
    New-Item -ItemType Directory -Path "$project\scripts","$project\target","$external\debug","$external\release","$fixture\sentinel" -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'maintenance.ps1') -Destination "$project\scripts\maintenance.ps1"
    $env:LOCALAPPDATA=Join-Path $fixture 'state'
    $script="$project\scripts\maintenance.ps1"
    Set-Content "$project\target\rebuild.txt" 'cache'
    Set-Content "$external\debug\rebuild.txt" 'cache'
    Set-Content "$external\release\keep.txt" 'release'
    Set-Content "$fixture\sentinel\keep.txt" 'sentinel'
    & pwsh -NoProfile -File $script -Deep -BuildCacheRoot $external
    Check ($LASTEXITCODE -eq 0 -and (Test-Path "$project\target\rebuild.txt")) 'preview does not delete'
    & pwsh -NoProfile -File $script -BudgetOnly -Apply -BudgetGiB 1 -BuildCacheRoot $external
    Check ($LASTEXITCODE -eq 0 -and (Test-Path "$external\debug\rebuild.txt")) 'under budget preserved'
    $junction="$project\target\unsafe"
    New-Item -ItemType Junction -Path $junction -Target "$fixture\sentinel" | Out-Null
    & pwsh -NoProfile -File $script -Deep -Apply 2>&1 | Out-Null
    Check ($LASTEXITCODE -ne 0 -and (Test-Path "$fixture\sentinel\keep.txt")) 'junction rejected and sentinel preserved'
    Remove-Item -LiteralPath $junction -Force
    $junction=$null
    & pwsh -NoProfile -File $script -Deep -Apply -BuildCacheRoot "$fixture\sentinel" 2>&1 | Out-Null
    Check ($LASTEXITCODE -ne 0 -and (Test-Path "$project\target\rebuild.txt")) 'invalid external scope rejects all deletion'
    Copy-Item -LiteralPath "$env:SystemRoot\System32\cmd.exe" -Destination "$external\debug\fixture.exe"
    $ownedProcess=Start-Process -FilePath "$external\debug\fixture.exe" -ArgumentList '/c ping -n 30 127.0.0.1 >nul' -WindowStyle Hidden -PassThru
    & pwsh -NoProfile -File $script -Deep -Apply -Scheduled -BuildCacheRoot $external
    $record=Get-Content "$env:LOCALAPPDATA\ZiDevTools\maintenance\last-run.json" -Raw | ConvertFrom-Json
    Check ($LASTEXITCODE -eq 0 -and $record.result -eq 'skipped-build-active' -and (Test-Path "$project\target\rebuild.txt")) 'running executable in cache skips entire cleanup'
    Stop-Process -Id $ownedProcess.Id -Force -ErrorAction SilentlyContinue
    $ownedProcess.WaitForExit()
    $ownedProcess=$null
    & pwsh -NoProfile -File $script -Deep -Apply -BuildCacheRoot $external
    Check ($LASTEXITCODE -eq 0 -and -not (Test-Path "$project\target") -and -not (Test-Path "$external\debug") -and (Test-Path "$external\release\keep.txt")) 'deep cleanup preserves release and sentinel'
    New-Item -ItemType Directory -Path "$project\target" -Force | Out-Null
    $stream=[IO.File]::OpenWrite("$project\target\budget.bin")
    try {$stream.SetLength(1GB+1)}finally{$stream.Dispose()}
    & pwsh -NoProfile -File $script -BudgetOnly -Apply -BudgetGiB 1
    Check ($LASTEXITCODE -eq 0 -and -not (Test-Path "$project\target")) 'over budget cleans rebuildable cache'
} finally {
    if($ownedProcess){Stop-Process -Id $ownedProcess.Id -Force -ErrorAction SilentlyContinue}
    if($junction -and (Test-Path -LiteralPath $junction)){Remove-Item -LiteralPath $junction -Force}
    $env:LOCALAPPDATA=$oldLocal
    $resolved=[IO.Path]::GetFullPath($fixture)
    if(-not $resolved.StartsWith($base+'\',[StringComparison]::OrdinalIgnoreCase) -or (Split-Path $resolved -Leaf) -notmatch '^zi-storage-test-[a-f0-9]{32}$'){throw 'Unsafe fixture cleanup path'}
    if(Test-Path -LiteralPath $resolved){Remove-Item -LiteralPath $resolved -Recurse -Force}
}
