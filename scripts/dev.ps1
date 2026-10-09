# Development entry: reuse caches without a scan or cleanup before Cargo.
# Examples: ./scripts/dev.ps1 (build), ./scripts/dev.ps1 dt-test
param([switch]$AllowBuildOverrides)
$ErrorActionPreference='Stop'
$CargoArguments=@($args)
if(-not $CargoArguments.Count){$CargoArguments=@('dt-build')}
if(-not $AllowBuildOverrides){
    $overrides=@(Get-ChildItem Env: | Where-Object {
        $_.Value -and ($_.Name -match '^CARGO_PROFILE_|^RUSTFLAGS$|^CARGO_ENCODED_RUSTFLAGS$|^CARGO_TARGET_DIR$|^CARGO_INCREMENTAL$|^RUSTC_WORKSPACE_WRAPPER$')
    })
    if($overrides.Count){
        throw ('Build overrides create another cache configuration: '+($overrides.Name -join ', ')+'. Use -AllowBuildOverrides for an intentional experiment.')
    }
}
Push-Location (Join-Path $PSScriptRoot '..')
$buildLock=[Threading.Mutex]::new($false,'Local\ZiDevToolsBuild')
$entered=$false
try {
    try{$entered=$buildLock.WaitOne(0)}catch [Threading.AbandonedMutexException]{$entered=$true}
    if(-not $entered){throw 'A build or cache cleanup already owns this workspace.'}
    & cargo @CargoArguments
    $code=$LASTEXITCODE
} finally {
    if($entered){$buildLock.ReleaseMutex()}
    $buildLock.Dispose()
    Pop-Location
}
exit $code
