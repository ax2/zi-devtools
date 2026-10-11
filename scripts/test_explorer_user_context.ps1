# Disposable hosted Windows CI only. Never create users on a developer machine.
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'This fixture requires a disposable Windows GitHub Actions runner.'
}
$testUser = 'ziCtx' + [Guid]::NewGuid().ToString('N').Substring(0, 12)
$testPassword = 'Zi!' + [Convert]::ToBase64String([Security.Cryptography.RandomNumberGenerator]::GetBytes(48)) + 'aA7'
$profileKeeper = $null
$created = $false
try {
    $secure = ConvertTo-SecureString $testPassword -AsPlainText -Force
    New-LocalUser -Name $testUser -Password $secure -AccountNeverExpires -PasswordNeverExpires | Out-Null
    $created = $true
    $credential = [Management.Automation.PSCredential]::new("$env:COMPUTERNAME\$testUser", $secure)
    $profileKeeper = Start-Process -FilePath "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" `
        -WorkingDirectory "$env:SystemRoot\System32" -Credential $credential -LoadUserProfile `
        -ArgumentList @('-NoProfile', '-NonInteractive', '-Command', 'New-Item HKCU:\Software\Classes -Force | Out-Null; Start-Sleep -Seconds 240') `
        -WindowStyle Hidden -PassThru
    $sid = (Get-LocalUser -Name $testUser).SID.Value
    $deadline = (Get-Date).AddSeconds(30)
    while (-not (Test-Path -LiteralPath "Registry::HKEY_USERS\${sid}_Classes")) {
        if ($profileKeeper.HasExited -or (Get-Date) -gt $deadline) { throw 'Disposable profile did not load.' }
        Start-Sleep -Milliseconds 200
    }
    # Credentials exist only in this process tree, never GITHUB_ENV or artifacts.
    $env:ZI_TEST_USER = $testUser
    $env:ZI_TEST_PASSWORD = $testPassword
    $testArgs = @('test', '-p', 'zi-shell-core', '--locked', '--', '--ignored', '--exact',
        'explorer::lifecycle::store::joint::tests::cross_account_joint_transaction_does_not_use_cached_process_hkcu')
    $testOutput = @(& cargo @testArgs 2>&1)
    $testExit = $LASTEXITCODE
    $testOutput | ForEach-Object { Write-Output $_.ToString() }
    if ($testExit -ne 0 -or ($testOutput -join "`n") -notmatch 'test result: ok\. 1 passed; 0 failed; 0 ignored;') {
        throw 'Cross-account registry fixture failed or did not execute exactly one test.'
    }
} finally {
    Remove-Item Env:ZI_TEST_USER, Env:ZI_TEST_PASSWORD -ErrorAction SilentlyContinue
    if ($null -ne $profileKeeper -and -not $profileKeeper.HasExited) { $profileKeeper.Kill(); $profileKeeper.WaitForExit() }
    if ($created) { Remove-LocalUser -Name $testUser }
    $testPassword = $null
    $secure = $null
    $credential = $null
    $sid = $null
}
