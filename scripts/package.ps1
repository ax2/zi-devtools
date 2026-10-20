param([string]$OutputDir = 'release')

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$projectRoot = (Get-Location).Path.TrimEnd('\')
$outputPath = [IO.Path]::GetFullPath((Join-Path $projectRoot $OutputDir))
if (-not $outputPath.StartsWith($projectRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'OutputDir must be inside the project directory'
}
$version = (Get-Content Cargo.toml | Select-String '^version = "([^"]+)"').Matches.Groups[1].Value
$msiVersion = $version.Split('-')[0]
if (!(Test-Path target/release/ZiDevTools.exe)) { throw 'Build release first' }
if (!(Test-Path target/release/ZiDevToolsMcp.exe)) { throw 'Build MCP server first' }
if (!(Test-Path target/release/zi_msi_actions.dll)) { throw 'Build release zi-msi-actions first' }
$inspector = 'target/release/examples/capability_inspect.exe'
if (!(Test-Path -LiteralPath $inspector)) { throw 'Build the read-only capability_inspect example first' }
$inspectionText = & $inspector 'target/release/ZiDevTools.exe'
if ($LASTEXITCODE -ne 0) { throw 'Desktop capability inspection failed' }
$inspection = $inspectionText | ConvertFrom-Json
if (!$inspection.declarationPresent -or !$inspection.declarationProperty -or !$inspection.sha256) {
    throw 'The desktop executable has no valid shell capability declaration'
}
$appExeSha256 = $inspection.sha256.ToLowerInvariant()
$appMenuCapabilities = $inspection.declarationProperty
$actualSha256 = (Get-FileHash -LiteralPath target/release/ZiDevTools.exe -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSha256 -ne $appExeSha256) { throw 'Desktop executable changed during capability inspection' }
$compiler = if ($env:ZIDEVTOOLS_WIX) { $env:ZIDEVTOOLS_WIX } else { 'wix' }
New-Item -ItemType Directory -Path $outputPath -Force | Out-Null
$msiPath = Join-Path $outputPath "ZiDevTools-$version-windows-x64.msi"
& $compiler build packaging/windows.wxs -arch x64 -d "AppVersion=$msiVersion" -d "AppExeSha256=$appExeSha256" -d "AppMenuCapabilities=$appMenuCapabilities" -d "MenuActionsDll=target/release/zi_msi_actions.dll" -o $msiPath
if ($LASTEXITCODE -ne 0) { throw 'MSI compilation failed' }
if ((Get-FileHash -LiteralPath target/release/ZiDevTools.exe -Algorithm SHA256).Hash.ToLowerInvariant() -ne $appExeSha256) {
    throw 'Desktop executable changed while the MSI was compiled'
}
$capabilityBase64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($appMenuCapabilities))
$manifestProof = Join-Path $outputPath 'ExplorerMenuManifestProof.json'
& python scripts/verify_msi_menu_manifest.py --msi $msiPath --exe target/release/ZiDevTools.exe --menu-actions-dll target/release/zi_msi_actions.dll --app-version $msiVersion --exe-sha256 $appExeSha256 --capabilities-base64 $capabilityBase64 --output $manifestProof
if ($LASTEXITCODE -ne 0) { throw 'Built MSI does not preserve the inspected shell capability manifest' }
Copy-Item target/release/ZiDevTools.exe (Join-Path $outputPath "ZiDevTools-$version-windows-x64.exe")
Copy-Item target/release/ZiDevToolsMcp.exe (Join-Path $outputPath "ZiDevToolsMcp-$version-windows-x64.exe")
Get-FileHash -LiteralPath $msiPath, (Join-Path $outputPath "ZiDevTools-$version-windows-x64.exe"), (Join-Path $outputPath "ZiDevToolsMcp-$version-windows-x64.exe") -Algorithm SHA256 | Sort-Object Path | ForEach-Object { '{0}  {1}' -f $_.Hash.ToLowerInvariant(), (Split-Path $_.Path -Leaf) } | Set-Content (Join-Path $outputPath 'SHA256SUMS.txt')
