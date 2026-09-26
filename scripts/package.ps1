$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$version = (Get-Content Cargo.toml | Select-String '^version = "([^"]+)"').Matches.Groups[1].Value
if (!(Test-Path target/release/ZiDevTools.exe)) { throw 'Build release first' }
$compiler = if ($env:ISCC) { $env:ISCC } else { "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" }
if (!(Test-Path $compiler)) { throw 'Install Inno Setup 6 before packaging' }
New-Item -ItemType Directory -Path release -Force | Out-Null
& $compiler "/DAppVersion=$version" packaging/windows.iss
if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed' }
Copy-Item target/release/ZiDevTools.exe "release/ZiDevTools-$version-windows-x64.exe"
Get-FileHash -Path "release/ZiDevTools-$version-windows-x64*.exe" -Algorithm SHA256 | Sort-Object Path | ForEach-Object { '{0}  {1}' -f $_.Hash.ToLowerInvariant(), (Split-Path $_.Path -Leaf) } | Set-Content release/SHA256SUMS.txt
