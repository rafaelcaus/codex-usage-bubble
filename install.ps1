<# Instala CODEX + CLAUDE sem administrador e sem login no GitHub.
   Execute pelo Instalar Widget.cmd do ZIP oficial. Nao baixa CLIs nem faz login.
#>
param([switch]$ValidateOnly)
$ErrorActionPreference='Stop'
$owner='rafaelcaus';$repo='codex-usage-bubble'
$assetName='claude-code-usage-bubble.exe'
$localExe=Join-Path $PSScriptRoot $assetName
$sumFile=Join-Path $PSScriptRoot 'SHA256SUMS.txt'
$tempDownload=$null
try {
    if ((Test-Path -LiteralPath $localExe) -and (Test-Path -LiteralPath $sumFile)) {
        $line=Get-Content -LiteralPath $sumFile | Where-Object {$_ -match '^([a-fA-F0-9]{64})\s+\*?claude-code-usage-bubble\.exe$'} | Select-Object -First 1
        if (!$line) {throw 'SHA256SUMS.txt invalido.'}
        $expected=($line -split '\s+')[0]
    } else {
        $release=Invoke-RestMethod -Uri "https://api.github.com/repos/$owner/$repo/releases/latest" -Headers @{'User-Agent'='codex-claude-widget-installer'}
        $asset=@($release.assets | Where-Object {$_.name -ceq $assetName})
        if ($asset.Count -ne 1) {throw 'Executavel oficial nao encontrado.'}
        if ($asset[0].digest -notmatch '^sha256:([a-fA-F0-9]{64})$') {throw 'Release sem SHA-256 verificavel.'}
        $expected=$Matches[1]
        $url=[string]$asset[0].browser_download_url
        if (!$url.StartsWith("https://github.com/$owner/$repo/releases/download/",[StringComparison]::Ordinal)) {throw 'Origem de download inesperada.'}
        $tempDownload=Join-Path ([IO.Path]::GetTempPath()) ('widget-'+[guid]::NewGuid().ToString('N')+'.exe')
        Invoke-WebRequest -Uri $url -OutFile $tempDownload -UseBasicParsing
        if ((Get-Item -LiteralPath $tempDownload).Length -ne $asset[0].size) {throw 'Download incompleto.'}
        $localExe=$tempDownload
    }
    $actual=(Get-FileHash -LiteralPath $localExe -Algorithm SHA256).Hash
    if ($actual -ne $expected) {throw 'Integridade SHA-256 divergente. Instalacao cancelada.'}
    Write-Host 'SHA-256 verificado.' -ForegroundColor Green
    if ($ValidateOnly) {return}
    $dir=Join-Path $env:LOCALAPPDATA 'ClaudeCodeUsageBubble'
    $cfgDir=Join-Path $env:APPDATA 'ClaudeCodeUsageBubble'
    New-Item -ItemType Directory -Path $dir,$cfgDir -Force|Out-Null
    $target=Join-Path $dir $assetName
    $cfg=Join-Path $cfgDir 'settings.json'
    $stamp=Get-Date -Format 'yyyyMMdd-HHmmss'
    # Preserve this PC's existing ChatGPT lifecycle supervisor if present.
    $supervisor=Get-ScheduledTask -TaskName 'Codex Bubble - acompanhar ChatGPT' -ErrorAction SilentlyContinue
    if ($supervisor) {Stop-ScheduledTask -TaskName $supervisor.TaskName}
    try {
        if (Test-Path -LiteralPath $target) {Copy-Item -LiteralPath $target -Destination ($target+'.bak-'+$stamp)}
        Get-Process -Name 'claude-code-usage-bubble' -ErrorAction SilentlyContinue | Where-Object {$_.Path -eq $target} | Stop-Process
        Start-Sleep -Seconds 1
        Copy-Item -LiteralPath $localExe -Destination $target -Force
        $cur=[pscustomobject]@{}
        if (Test-Path -LiteralPath $cfg) {
            Copy-Item -LiteralPath $cfg -Destination ($cfg+'.bak-'+$stamp)
            # Do not silently discard invalid existing preferences.
            $cur=Get-Content -LiteralPath $cfg -Raw|ConvertFrom-Json
        }
        foreach($setting in @{show_codex=$true;show_claude_code=$true;show_opencode_go=$false;update_check_interval_secs=3600;hourly_update_migrated=$true}.GetEnumerator()) {
            $cur|Add-Member -NotePropertyName $setting.Key -NotePropertyValue $setting.Value -Force
        }
        foreach($setting in @{poll_interval_ms=60000;bubble_size_logical=130;widget_visible=$true;only_over_chatgpt=$false;visible_sections=31}.GetEnumerator()) {
            if (!$cur.PSObject.Properties[$setting.Key]) {$cur|Add-Member -NotePropertyName $setting.Key -NotePropertyValue $setting.Value}
        }
        $cur|ConvertTo-Json -Depth 8|Set-Content -LiteralPath $cfg -Encoding UTF8
        if (!$supervisor) {
            New-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'ClaudeCodeUsageBubble' -Value ('"'+$target+'"') -PropertyType String -Force|Out-Null
            Start-Process -FilePath $target -WindowStyle Hidden
        }
        $shell=New-Object -ComObject WScript.Shell
        $shortcut=$shell.CreateShortcut((Join-Path ([Environment]::GetFolderPath('Desktop')) 'CODEX e CLAUDE Widget.lnk'))
        $shortcut.TargetPath=$target;$shortcut.WorkingDirectory=$dir;$shortcut.Save()
        Write-Host 'Instalado: CODEX + CLAUDE. Atualizacoes automaticas a cada hora ativas.' -ForegroundColor Green
        Write-Host 'Cada pessoa usa suas proprias contas. Se necessario, instale os CLIs oficiais e faca login com codex login e claude.'
        Write-Host 'Botao direito > Abrir configuracoes > Informacoes: escolha o que mostrar.'
        Write-Host 'Sem login no GitHub; sem permissao de escrita no repositorio.'
    } finally {if($supervisor){Start-ScheduledTask -TaskName $supervisor.TaskName}}
} finally {if($tempDownload -and (Test-Path -LiteralPath $tempDownload)){Remove-Item -LiteralPath $tempDownload}}
