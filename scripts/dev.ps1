# Development entry: reuse caches without a scan or cleanup before Cargo.
# Examples: ./scripts/dev.ps1 (build), ./scripts/dev.ps1 dt-test
param([switch]$AllowBuildOverrides, [switch]$UseUserCache)
$ErrorActionPreference='Stop'
$CargoArguments=@($args)
if(-not $CargoArguments.Count){$CargoArguments=@('dt-build')}
$userCacheRoot=$null
if($UseUserCache){
    if(-not $env:LOCALAPPDATA){throw 'LOCALAPPDATA is not set; cannot locate the user-local Cargo cache.'}
    $userCacheRoot=[IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'ZiDevTools\zi-devtools')).TrimEnd('\')
    if((Split-Path $userCacheRoot -Leaf) -ne 'zi-devtools'){throw 'User cache path must end in zi-devtools.'}
    $pathToCheck=$userCacheRoot
    while($pathToCheck -and -not (Test-Path -LiteralPath $pathToCheck)){
        $parent=Split-Path $pathToCheck -Parent
        if($parent -eq $pathToCheck){$pathToCheck=$null;break}
        $pathToCheck=$parent
    }
    if(-not $pathToCheck){throw 'Cannot locate an existing parent for the user-local cache.'}
    $cursor=Get-Item -LiteralPath $pathToCheck -Force
    while($cursor){
        if($cursor.Attributes -band [IO.FileAttributes]::ReparsePoint){throw "Linked user cache parent rejected: $($cursor.FullName)"}
        $cursor=$cursor.Parent
    }
    $existingCache=Get-Item -LiteralPath $userCacheRoot -Force -ErrorAction SilentlyContinue
    if($existingCache -and -not $existingCache.PSIsContainer){throw "User cache is not a directory: $userCacheRoot"}
    if($existingCache -and ($existingCache.Attributes -band [IO.FileAttributes]::ReparsePoint)){
        throw "Linked user cache rejected: $userCacheRoot"
    }
}
if(-not $AllowBuildOverrides){
    $overrides=@(Get-ChildItem Env: | Where-Object {
        $_.Value -and ($_.Name -match '^CARGO_PROFILE_|^RUSTFLAGS$|^CARGO_ENCODED_RUSTFLAGS$|^CARGO_INCREMENTAL$|^RUSTC_WORKSPACE_WRAPPER$' -or ($_.Name -eq 'CARGO_TARGET_DIR' -and -not $UseUserCache))
    })
    if($overrides.Count){
        throw ('Build overrides create another cache configuration: '+($overrides.Name -join ', ')+'. Use -AllowBuildOverrides for an intentional experiment.')
    }
}
Push-Location (Join-Path $PSScriptRoot '..')
$buildLock=[Threading.Mutex]::new($false,'Local\ZiDevToolsBuild')
$entered=$false
$targetDirWasSet=Test-Path Env:CARGO_TARGET_DIR
$previousTargetDir=$env:CARGO_TARGET_DIR
$userCacheApplied=$false
try {
    try{$entered=$buildLock.WaitOne(0)}catch [Threading.AbandonedMutexException]{$entered=$true}
    if(-not $entered){throw 'A build or cache cleanup already owns this workspace.'}
    if($UseUserCache){$env:CARGO_TARGET_DIR=$userCacheRoot;$userCacheApplied=$true}
    & cargo @CargoArguments
    $code=$LASTEXITCODE
} finally {
    if($userCacheApplied){
        if($targetDirWasSet){$env:CARGO_TARGET_DIR=$previousTargetDir}else{Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue}
    }
    if($entered){$buildLock.ReleaseMutex()}
    $buildLock.Dispose()
    Pop-Location
}
exit $code
