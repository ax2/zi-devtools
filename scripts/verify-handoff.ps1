# Verify native handoff and shared tool-entry routing before an optimized build.
# All service/configuration data comes from disposable native UI fixtures.
param([switch]$Release, [switch]$PlanOnly)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..')).TrimEnd('\')
$targetRoot = Join-Path $projectRoot 'target'
$cacheRoot = Join-Path $env:LOCALAPPDATA 'ZiDevTools\zi-devtools'
$stages = @(
    @{ Name='format'; Kind='cargo'; Arguments=@('fmt','--all','--check') },
    @{ Name='handoff-tests'; Kind='cargo'; Arguments=@('test','--lib','app::handoff','--locked') },
    @{ Name='image-entry-tests'; Kind='cargo'; Arguments=@('test','--lib','image_tools::entry','--locked') },
    @{ Name='image-route-catalog'; Kind='cargo'; Arguments=@('test','--lib','every_registered_image_entry','--locked') },
    @{ Name='provenance-tests'; Kind='cargo'; Arguments=@('test','--lib','tool_provenance','--locked') },
    @{ Name='launcher-keyboard-tests'; Kind='cargo'; Arguments=@('test','--lib','launcher::keyboard','--locked') },
    @{ Name='history-tests'; Kind='cargo'; Arguments=@('test','--lib','app::history','--locked') },
    @{ Name='instance-search-tests'; Kind='cargo'; Arguments=@('test','--lib','instance_navigation','--locked') },
    @{ Name='ui-build'; Kind='cargo'; Arguments=@('build','--example','handoff_access_ui','--example','handoff_scroll_ui','--example','handoff_keyboard_ui','--example','image_entry_ui','--example','launcher_keyboard_ui','--example','search_scopes_ui','--example','navigation_history_ui','--example','navigation_extended_ui','--example','instance_search_ui','--features','ui-preview','--locked') },
    @{ Name='access-light'; Kind='ui'; Example='handoff_access_ui'; Arguments=@('light','visits') },
    @{ Name='access-dark'; Kind='ui'; Example='handoff_access_ui'; Arguments=@('dark','visits') },
    @{ Name='scroll-light'; Kind='ui'; Example='handoff_scroll_ui'; Arguments=@('light','csv') },
    @{ Name='scroll-dark'; Kind='ui'; Example='handoff_scroll_ui'; Arguments=@('dark','tsv') },
    @{ Name='keyboard-light'; Kind='ui'; Example='handoff_keyboard_ui'; Arguments=@('light') },
    @{ Name='keyboard-dark'; Kind='ui'; Example='handoff_keyboard_ui'; Arguments=@('dark') },
    @{ Name='arrows-light'; Kind='ui'; Example='handoff_keyboard_ui'; Arguments=@('light','arrows') },
    @{ Name='arrows-dark'; Kind='ui'; Example='handoff_keyboard_ui'; Arguments=@('dark','arrows') },
    @{ Name='image-entry-light'; Kind='ui'; Example='image_entry_ui'; Arguments=@('light') },
    @{ Name='image-entry-dark'; Kind='ui'; Example='image_entry_ui'; Arguments=@('dark') },
    @{ Name='launcher-main-light'; Kind='ui'; Example='launcher_keyboard_ui'; Arguments=@('light','main') },
    @{ Name='launcher-main-dark'; Kind='ui'; Example='launcher_keyboard_ui'; Arguments=@('dark','main') },
    @{ Name='launcher-quick-light'; Kind='ui'; Example='launcher_keyboard_ui'; Arguments=@('light','quick') },
    @{ Name='launcher-quick-dark'; Kind='ui'; Example='launcher_keyboard_ui'; Arguments=@('dark','quick') },
    @{ Name='search-scopes-light'; Kind='ui'; Example='search_scopes_ui'; Arguments=@('light') },
    @{ Name='search-scopes-dark'; Kind='ui'; Example='search_scopes_ui'; Arguments=@('dark') },
    @{ Name='history-light'; Kind='ui'; Example='navigation_history_ui'; Arguments=@('light') },
    @{ Name='history-dark'; Kind='ui'; Example='navigation_history_ui'; Arguments=@('dark') },
    @{ Name='extended-light'; Kind='ui'; Example='navigation_extended_ui'; Arguments=@('light','extended') },
    @{ Name='extended-dark'; Kind='ui'; Example='navigation_extended_ui'; Arguments=@('dark','extended') },
    @{ Name='handoff-history-light'; Kind='ui'; Example='navigation_extended_ui'; Arguments=@('light','handoff') },
    @{ Name='handoff-history-dark'; Kind='ui'; Example='navigation_extended_ui'; Arguments=@('dark','handoff') },
    @{ Name='task-history-light'; Kind='ui'; Example='navigation_extended_ui'; Arguments=@('light','tasks') },
    @{ Name='task-history-dark'; Kind='ui'; Example='navigation_extended_ui'; Arguments=@('dark','tasks') },
    @{ Name='instances-main-light'; Kind='ui'; Example='instance_search_ui'; Arguments=@('light','main') },
    @{ Name='instances-main-dark'; Kind='ui'; Example='instance_search_ui'; Arguments=@('dark','main') },
    @{ Name='instances-quick-light'; Kind='ui'; Example='instance_search_ui'; Arguments=@('light','quick') },
    @{ Name='instances-quick-dark'; Kind='ui'; Example='instance_search_ui'; Arguments=@('dark','quick') },
    @{ Name='workspace-tests'; Kind='cargo'; Arguments=@('test','--workspace','--locked') },
    @{ Name='clippy'; Kind='cargo'; Arguments=@('dt-lint') }
)
if ($Release) { $stages += @{ Name='release'; Kind='cargo'; Arguments=@('build','--release','--locked') } }
if ($PlanOnly) {
    [ordered]@{ Project=$projectRoot; Cache=$cacheRoot; ReleaseRequested=[bool]$Release; Stages=$stages } | ConvertTo-Json -Depth 6
    exit 0
}
# Do not follow a junction into an unrelated output tree. No cleanup is performed.
$cursor = Get-Item -LiteralPath $projectRoot -Force
while ($cursor) {
    if ($cursor.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Linked project parent rejected: $($cursor.FullName)" }
    $cursor = $cursor.Parent
}
if (Test-Path -LiteralPath $targetRoot) {
    $item = Get-Item -LiteralPath $targetRoot -Force
    if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Unsafe target output root' }
}
$output = Join-Path $targetRoot ('handoff-verify-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0,8))
New-Item -ItemType Directory -Path $output | Out-Null
$results = [Collections.Generic.List[object]]::new()
$report = [ordered]@{ Status='running'; ReleaseRequested=[bool]$Release; Stages=$results; Published=$false }
Push-Location $projectRoot
try {
    foreach ($stage in $stages) {
        $timer = [Diagnostics.Stopwatch]::StartNew()
        $log = Join-Path $output ($stage.Name + '.log')
        $stageArguments = $stage.Arguments
        Write-Output ("Checking " + $stage.Name)
        if ($stage.Kind -eq 'cargo') {
            & (Join-Path $PSScriptRoot 'dev.ps1') -UseUserCache @stageArguments *> $log
        } else {
            $executable = Join-Path $cacheRoot ('debug\examples\' + $stage.Example + '.exe')
            & $executable (Join-Path $output $stage.Name) @stageArguments *> $log
        }
        $code = $LASTEXITCODE
        $results.Add([ordered]@{ Name=$stage.Name; ExitCode=$code; Seconds=[math]::Round($timer.Elapsed.TotalSeconds,2); Log=$log })
        if ($code -ne 0) { throw "Stage $($stage.Name) failed; later stages were not run. See $log" }
    }
    $report.Status = 'passed'
} catch {
    $report.Status = 'failed'
    throw
} finally {
    $report | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $output 'checks.json') -Encoding utf8
    Pop-Location
    Write-Output ("Verification record: " + $output)
}
