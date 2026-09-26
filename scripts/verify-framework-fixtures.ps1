param(
    [Parameter(Mandatory=$true)][string]$JavaHome,
    [Parameter(Mandatory=$true)][string]$Python,
    [Parameter(Mandatory=$true)][string]$ComparisonPython
)
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ('zi-framework-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $fixtureRoot | Out-Null
Copy-Item -LiteralPath (Join-Path $repoRoot 'tests/fixtures/framework/Fixture.java') -Destination $fixtureRoot
& $Python (Join-Path $repoRoot 'tests/fixtures/framework/django_fixture.py') $fixtureRoot
if ($LASTEXITCODE -ne 0) { throw 'Disposable Django fixture failed' }
$javaExe = Join-Path $JavaHome 'bin/java.exe'
& (Join-Path $JavaHome 'bin/javac.exe') '-encoding' 'UTF-8' (Join-Path $fixtureRoot 'Fixture.java')
if ($LASTEXITCODE -ne 0) { throw 'Disposable Java fixture compilation failed' }
$forwardRoot = $fixtureRoot.Replace('\','/')
$javaArgs = @('-Xmx128m', "-XX:StartFlightRecording=filename=$forwardRoot/fixture.jfr,duration=3s,settings=profile", "-Xlog:gc:file=$forwardRoot/gc.log", '-cp', $fixtureRoot, 'Fixture')
# Start-Process joins arguments; quote each known argument to preserve spaces in paths.
$quotedArgs = ($javaArgs | ForEach-Object { '"' + $_ + '"' }) -join ' '
$fixtureProcess = Start-Process -FilePath $javaExe -ArgumentList $quotedArgs -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $fixtureRoot 'java.log') -RedirectStandardError (Join-Path $fixtureRoot 'java.err')
try {
    Start-Sleep -Seconds 2
    & (Join-Path $JavaHome 'bin/jcmd.exe') $fixtureProcess.Id 'Thread.print' | Set-Content -Encoding utf8 (Join-Path $fixtureRoot 'threads.txt')
    if ($LASTEXITCODE -ne 0) { throw 'Disposable thread dump failed' }
    if (!$fixtureProcess.WaitForExit(12000)) { throw 'Disposable Java fixture timed out' }
    if ($fixtureProcess.ExitCode -ne 0) { throw 'Disposable Java fixture failed' }
} finally {
    $fixtureProcess.Refresh()
    if (!$fixtureProcess.HasExited) { Stop-Process -Id $fixtureProcess.Id }
}
& cargo run --manifest-path (Join-Path $repoRoot 'Cargo.toml') --example framework_verify -- $fixtureRoot $javaExe $Python $ComparisonPython (Join-Path $JavaHome 'bin/jfr.exe')
if ($LASTEXITCODE -ne 0) { throw 'Framework report assertions failed' }
Write-Output "Fixture evidence retained at: $fixtureRoot"
