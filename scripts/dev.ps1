# Development entry: reuse caches without a scan or cleanup before Cargo.
# Example: ./scripts/dev.ps1 test --locked
$ErrorActionPreference='Stop'
if(-not $args.Count){throw 'Supply a Cargo command, e.g. test --locked.'}
Push-Location (Join-Path $PSScriptRoot '..')
try {
    & cargo @args
    $code=$LASTEXITCODE
} finally {Pop-Location}
exit $code
