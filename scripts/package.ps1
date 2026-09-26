$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$version = (Get-Content Cargo.toml | Select-String '^version = "([^"]+)"').Matches.Groups[1].Value
if (!(Test-Path target/release/ZiDevTools.exe)) { throw 'Build release first' }
$compiler = if ($env:WIX) { $env:WIX } else { 'wix' }
New-Item -ItemType Directory -Path release -Force | Out-Null
& $compiler build packaging/windows.wxs -arch x64 -d "AppVersion=$version" -o "release/ZiDevTools-$version-windows-x64.msi"
if ($LASTEXITCODE -ne 0) { throw 'MSI compilation failed' }
Copy-Item target/release/ZiDevTools.exe "release/ZiDevTools-$version-windows-x64.exe"
Get-FileHash -LiteralPath "release/ZiDevTools-$version-windows-x64.msi", "release/ZiDevTools-$version-windows-x64.exe" -Algorithm SHA256 | Sort-Object Path | ForEach-Object { '{0}  {1}' -f $_.Hash.ToLowerInvariant(), (Split-Path $_.Path -Leaf) } | Set-Content release/SHA256SUMS.txt
