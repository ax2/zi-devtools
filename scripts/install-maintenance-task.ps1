$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).ProviderPath
$scriptPath = Join-Path $PSScriptRoot 'maintenance.ps1'
$shell = (Get-Command pwsh -ErrorAction Stop).Source
$user = [Security.Principal.WindowsIdentity]::GetCurrent().Name
$taskName = 'ZiDevTools-WeeklyMaintenance'

$action = New-ScheduledTaskAction -Execute $shell -Argument ('-NoProfile -NonInteractive -File "{0}" -Apply -Scheduled' -f $scriptPath) -WorkingDirectory $projectRoot
$trigger = New-ScheduledTaskTrigger -Weekly -DaysOfWeek Sunday -At '03:00'
$principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet -StartWhenAvailable -ExecutionTimeLimit (New-TimeSpan -Minutes 20)
Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null
$task = Get-ScheduledTask -TaskName $taskName
Write-Output ("{0}: {1}; 下次运行 {2}" -f $task.TaskName, $task.State, (Get-ScheduledTaskInfo -TaskName $taskName).NextRunTime)
