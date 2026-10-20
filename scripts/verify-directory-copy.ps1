# Verify directory task and copy interaction before spending time on an optimized build.
# All service/configuration data comes from disposable native UI fixtures.
param([switch]$Release, [switch]$PlanOnly)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..')).TrimEnd('\')
$targetRoot = Join-Path $projectRoot 'target'
$cacheRoot = Join-Path $env:LOCALAPPDATA 'ZiDevTools\zi-devtools'
$stages = @(
    @{ Name='format'; Kind='cargo'; Arguments=@('fmt','--all','--check') },
    @{ Name='directory-tests'; Kind='cargo'; Arguments=@('test','--lib','directory_compare','--locked') },
    @{ Name='ui-build'; Kind='cargo'; Arguments=@('build','--example','directory_tasks_ui','--example','directory_retry_ui','--features','ui-preview','--locked') },
    @{ Name='tasks-light'; Kind='ui'; Example='directory_tasks_ui'; Arguments=@('light') },
    @{ Name='tasks-dark'; Kind='ui'; Example='directory_tasks_ui'; Arguments=@('dark') },
    @{ Name='retry-light'; Kind='ui'; Example='directory_retry_ui'; Arguments=@('light') },
    @{ Name='retry-dark'; Kind='ui'; Example='directory_retry_ui'; Arguments=@('dark') },
    @{ Name='progress-light'; Kind='ui'; Example='directory_tasks_ui'; Arguments=@('light','progress') },
    @{ Name='progress-dark'; Kind='ui'; Example='directory_tasks_ui'; Arguments=@('dark','progress') },
    @{ Name='layout-narrow-light'; Kind='ui'; Example='directory_tasks_ui'; Arguments=@('light','layout-narrow') },
    @{ Name='layout-narrow-dark'; Kind='ui'; Example='directory_tasks_ui'; Arguments=@('dark','layout-narrow') },
    @{ Name='layout-wide-light'; Kind='ui'; Example='directory_tasks_ui'; Arguments=@('light','layout-wide') },
    @{ Name='layout-wide-dark'; Kind='ui'; Example='directory_tasks_ui'; Arguments=@('dark','layout-wide') },
    @{ Name='layout-compact-light'; Kind='ui'; Example='directory_tasks_ui'; Arguments=@('light','layout-compact') },
    @{ Name='layout-compact-dark'; Kind='ui'; Example='directory_tasks_ui'; Arguments=@('dark','layout-compact') },
    @{ Name='pagination-light'; Kind='ui'; Example='directory_tasks_ui'; Arguments=@('light','pagination') },
    @{ Name='pagination-dark'; Kind='ui'; Example='directory_tasks_ui'; Arguments=@('dark','pagination') },
    @{ Name='results-light'; Kind='ui'; Example='directory_tasks_ui'; Arguments=@('light','results') },
    @{ Name='results-dark'; Kind='ui'; Example='directory_tasks_ui'; Arguments=@('dark','results') },
    @{ Name='transfer-light'; Kind='ui'; Example='directory_tasks_ui'; Arguments=@('light','transfer') },
    @{ Name='transfer-dark'; Kind='ui'; Example='directory_tasks_ui'; Arguments=@('dark','transfer') },
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
$output = Join-Path $targetRoot ('directory-verify-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0,8))
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
