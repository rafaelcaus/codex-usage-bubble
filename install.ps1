<# Instalador automatico do fork Codex Usage Bubble.
   Repositorio: https://github.com/rafaelcaus/codex-usage-bubble

   O que faz (sem precisar de admin):
     1. Confere Node/npm e o Codex CLI (instala via npm se faltar).
     2. Verifica o login do Codex (NÃO faz login sozinho: exige interacao).
     3. Baixa o .exe da latest release DESTE repositorio (GitHub oficial).
     4. Valida tamanho + SHA-256 contra os dados da API do GitHub.
     5. Instala em %LOCALAPPDATA%\ClaudeCodeUsageBubble\.
     6. Configura Codex=ON, Claude=OFF (com backup do settings.json).
     7. Registra "Start with Windows" (mesmo mecanismo nativo do app).
     8. Inicia a bolha e imprime o relatorio de validacao.

   Uso (PowerShell):
     irm https://raw.githubusercontent.com/rafaelcaus/codex-usage-bubble/main/install.ps1 | iex
   Ou baixe este arquivo e execute localmente.

   NÃO exibe nem grava tokens: apenas verifica o STATUS do login.
#>
# 'Continue' de proposito (nao 'Stop'): o shim codex.ps1 escreve o status
# no stderr e o PowerShell trataria como erro fatal. Falhas reais usam
# `throw` explicito abaixo.
$ErrorActionPreference = 'Continue'
$Owner = 'rafaelcaus'
$Repo  = 'codex-usage-bubble'

function Ok($m)   { Write-Host "[OK] $m" -ForegroundColor Green }
function Info($m) { Write-Host "-- $m" }
function Warn($m) { Write-Host "[AVISO] $m" -ForegroundColor Yellow }

Info '1/7 Codex CLI'
$codex = Get-Command codex -ErrorAction SilentlyContinue
if (-not $codex) {
    if (Get-Command npm -ErrorAction SilentlyContinue) {
        Info 'Codex CLI ausente; instalando via npm...'
        npm install -g @openai/codex
        $codex = Get-Command codex -ErrorAction SilentlyContinue
    }
    if (-not $codex) { throw 'Codex CLI nao encontrado e nao foi possivel instalar via npm. Instale Node LTS + Codex CLI e rode de novo.' }
}
codex --version
Info 'Status do login (se pedir login, rode `codex login` manualmente depois):'
codex login status 2>&1 | Select-Object -First 3

Info '2/7 Latest release deste repositorio'
$rel = Invoke-RestMethod -Uri "https://api.github.com/repos/$Owner/$Repo/releases/latest" `
    -Headers @{ 'User-Agent' = 'codex-usage-bubble-installer' }
"Versao: $($rel.tag_name)"
$asset = $rel.assets | Where-Object { $_.name -eq 'claude-code-usage-bubble.exe' } | Select-Object -First 1
if (-not $asset) { $asset = $rel.assets | Where-Object { $_.name -like '*.exe' } | Select-Object -First 1 }
if (-not $asset) { throw 'Nenhum .exe na latest release.' }
"Asset: $($asset.name) ($($asset.size) bytes)"

Info '3/7 Download + validacao'
$dir = "$env:LOCALAPPDATA\ClaudeCodeUsageBubble"
New-Item -ItemType Directory -Path $dir -Force | Out-Null
$exe = "$dir\claude-code-usage-bubble.exe"
if (Test-Path $exe) {
    $bak = "$dir\claude-code-usage-bubble.exe.bak-$(Get-Date -Format yyyyMMdd-HHmmss)"
    Copy-Item $exe $bak -Force
    "Backup do exe anterior: $bak"
}
Info 'Parando instancia em execucao (o exe trava o arquivo)...'
Get-Process 'claude-code-usage-bubble' -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $exe -UseBasicParsing
$got = (Get-Item $exe).Length
if ($got -ne $asset.size) { throw "Tamanho divergente: esperado $($asset.size), obtido $got." }
Ok "Tamanho confere ($got bytes)"
$sha = (Get-FileHash $exe -Algorithm SHA256).Hash
"SHA-256 local: $sha"
if ($asset.digest -match '^sha256:([0-9a-fA-F]{64})$') {
    if ($sha -ne $Matches[1].ToUpper()) { throw 'SHA-256 DIVERGENTE. Apague o arquivo e tente de novo.' }
    Ok 'SHA-256 confere com o digest oficial do GitHub.'
} else {
    Warn 'Release sem digest oficial; validado apenas por origem + tamanho.'
}

Info '4/7 Configuracao (somente Codex)'
$cfgDir = "$env:APPDATA\ClaudeCodeUsageBubble"
New-Item -ItemType Directory -Path $cfgDir -Force | Out-Null
$cfg = "$cfgDir\settings.json"
if (Test-Path $cfg) {
    Copy-Item $cfg "$cfg.bak-$(Get-Date -Format yyyyMMdd-HHmmss)" -Force
    Info 'Backup do settings.json criado.'
}
$cur = New-Object PSObject
if (Test-Path $cfg) {
    try { $cur = Get-Content $cfg -Raw | ConvertFrom-Json } catch { $cur = New-Object PSObject }
}
$cur | Add-Member -NotePropertyName 'show_claude_code' -NotePropertyValue $false -Force
$cur | Add-Member -NotePropertyName 'show_codex' -NotePropertyValue $true -Force
$cur | Add-Member -NotePropertyName 'show_opencode_go' -NotePropertyValue $false -Force
function Ensure-Setting($name, $value) {
    if (-not ($cur | Get-Member -Name $name -MemberType NoteProperty)) {
        $cur | Add-Member -NotePropertyName $name -NotePropertyValue $value
    }
}
Ensure-Setting 'poll_interval_ms' 60000
Ensure-Setting 'bubble_size_logical' 50
Ensure-Setting 'only_over_chatgpt' $true
Ensure-Setting 'widget_visible' $true
Ensure-Setting 'update_check_interval_secs' 86400
($cur | ConvertTo-Json -Depth 5) | Set-Content $cfg
Ok 'Providers: Codex ON, Claude OFF.'

Info '5/7 Start with Windows (mecanismo nativo: HKCU Run)'
New-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' `
    -Name 'ClaudeCodeUsageBubble' -Value "`"$exe`"" -PropertyType String -Force | Out-Null
Ok 'Entrada unica de inicializacao registrada.'

Info '6/7 Iniciando a bolha'
Start-Process -FilePath $exe
Start-Sleep -Seconds 5
$p = Get-Process 'claude-code-usage-bubble' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $p) { throw 'O processo nao subiu. Veja o menu/SmartScreen.' }
Ok "Rodando (PID $($p.Id))."

Info '7/7 Validacao'
if ((Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'ClaudeCodeUsageBubble' -ErrorAction SilentlyContinue)) { Ok 'Startup OK.' }
"Pronto! Se o SmartScreen pedir confirmacao (binario sem assinatura): Mais informacoes -> Executar assim mesmo."
"Botao direito na bolha: confira Providers (so Codex) e arraste para a lateral. O auto-update Daily mantem este fork atualizado sozinho."
