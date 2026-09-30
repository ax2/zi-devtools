param([string]$OutputDir = 'release')

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$projectRoot = (Get-Location).Path.TrimEnd('\')
$outputPath = [IO.Path]::GetFullPath((Join-Path $projectRoot $OutputDir))
if (-not $outputPath.StartsWith($projectRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'OutputDir must be inside the project directory'
}
$version = (Get-Content Cargo.toml | Select-String '^version = "([^"]+)"').Matches.Groups[1].Value
if (!(Test-Path target/release/ZiDevTools.exe)) { throw 'Build release first' }
if (!(Test-Path target/release/ZiDevToolsMcp.exe)) { throw 'Build MCP server first' }
$compiler = if ($env:ZIDEVTOOLS_WIX) { $env:ZIDEVTOOLS_WIX } else { 'wix' }
New-Item -ItemType Directory -Path $outputPath -Force | Out-Null
& $compiler build packaging/windows.wxs -arch x64 -d "AppVersion=$version" -o (Join-Path $outputPath "ZiDevTools-$version-windows-x64.msi")
if ($LASTEXITCODE -ne 0) { throw 'MSI compilation failed' }
Copy-Item target/release/ZiDevTools.exe (Join-Path $outputPath "ZiDevTools-$version-windows-x64.exe")
Copy-Item target/release/ZiDevToolsMcp.exe (Join-Path $outputPath "ZiDevToolsMcp-$version-windows-x64.exe")
Get-FileHash -LiteralPath (Join-Path $outputPath "ZiDevTools-$version-windows-x64.msi"), (Join-Path $outputPath "ZiDevTools-$version-windows-x64.exe"), (Join-Path $outputPath "ZiDevToolsMcp-$version-windows-x64.exe") -Algorithm SHA256 | Sort-Object Path | ForEach-Object { '{0}  {1}' -f $_.Hash.ToLowerInvariant(), (Split-Path $_.Path -Leaf) } | Set-Content (Join-Path $outputPath 'SHA256SUMS.txt')
