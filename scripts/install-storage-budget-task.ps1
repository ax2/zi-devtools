param([string]$BuildCacheRoot, [ValidateRange(0,1024)][double]$BudgetGiB=0)
$ErrorActionPreference='Stop'
$scriptPath=Join-Path $PSScriptRoot 'maintenance.ps1'
$shell=(Get-Command pwsh).Source
$user=[Security.Principal.WindowsIdentity]::GetCurrent().Name
# Keep the installed task name, but record only; no capacity-triggered deletion.
$arguments='-NoProfile -NonInteractive -File "{0}" -Apply -Scheduled -BudgetOnly -BudgetGiB {1}' -f $scriptPath,$BudgetGiB
if($BuildCacheRoot){
    if($BuildCacheRoot.Contains('"')){throw 'Invalid cache path'}
    # Validate scope before registering an audit. The legacy task name is retained.
    & $shell -NoProfile -File $scriptPath -BudgetOnly -BudgetGiB $BudgetGiB -BuildCacheRoot $BuildCacheRoot
    if($LASTEXITCODE -ne 0){throw 'Cache safety validation failed'}
    $arguments+=' -BuildCacheRoot "{0}"' -f ([IO.Path]::GetFullPath($BuildCacheRoot))
}
$action=New-ScheduledTaskAction -Execute $shell -Argument $arguments -WorkingDirectory (Split-Path $PSScriptRoot)
$trigger=New-ScheduledTaskTrigger -Daily -At '03:30'
$principal=New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited
$settings=New-ScheduledTaskSettingsSet -StartWhenAvailable -ExecutionTimeLimit (New-TimeSpan -Minutes 20) -MultipleInstances IgnoreNew
Register-ScheduledTask -TaskName ZiDevTools-DailyStorageBudget -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null
Get-ScheduledTaskInfo -TaskName ZiDevTools-DailyStorageBudget | Select-Object NextRunTime,LastTaskResult
