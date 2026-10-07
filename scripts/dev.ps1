# Development entry: enforce the idle cache budget before invoking Cargo.
# Example: ./scripts/dev.ps1 test --locked
$ErrorActionPreference='Stop'
if(-not $args.Count){throw 'Supply a Cargo command, e.g. test --locked.'}
$maintenance=Join-Path $PSScriptRoot 'maintenance.ps1'
$options=@('-NoProfile','-File',$maintenance,'-Apply','-BudgetOnly')
if($env:CARGO_TARGET_DIR){
    $cache=[IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
    $project=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
    if($cache -ne (Join-Path $project 'target')){$options+=@('-BuildCacheRoot',$cache)}
}
& (Get-Command pwsh).Source @options
if($LASTEXITCODE -ne 0){throw 'Storage preflight failed; Cargo was not started.'}
$env:CARGO_INCREMENTAL='0'
Push-Location (Join-Path $PSScriptRoot '..')
try {
    & cargo @args
    $code=$LASTEXITCODE
} finally {Pop-Location}
exit $code
