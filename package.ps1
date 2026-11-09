param([string]$OutputDir=(Join-Path $PSScriptRoot 'dist'))
$ErrorActionPreference='Stop'
$exe=Join-Path $PSScriptRoot 'target\release\claude-code-usage-bubble.exe'
if (!(Test-Path -LiteralPath $exe)) {throw 'Compile cargo build --release primeiro.'}
New-Item -ItemType Directory -Path $OutputDir -Force|Out-Null
$stage=Join-Path $OutputDir ('widget-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $stage|Out-Null
foreach($name in @('install.ps1','Instalar Widget.cmd','COMO INSTALAR.md','LICENSE','SECURITY.md')) {Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination $stage}
Copy-Item -LiteralPath $exe -Destination $stage
$hash=(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLower()
$hash+'  claude-code-usage-bubble.exe'|Set-Content -LiteralPath (Join-Path $stage 'SHA256SUMS.txt') -Encoding ASCII
$zip=Join-Path $OutputDir 'Codex-Claude-Widget-Windows.zip'
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -Force
Write-Output $zip
