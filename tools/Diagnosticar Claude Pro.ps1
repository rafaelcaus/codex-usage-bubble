# Diagnostico Claude Pro: consulta somente leitura no host oficial, sem prompts.
$ErrorActionPreference='Stop'
$report=New-Object System.Collections.Generic.List[string]
function Record($name,$value){$report.Add($name+': '+[string]$value)}
Record 'Versao do diagnostico' '1'
$root=if($env:CLAUDE_CONFIG_DIR){$env:CLAUDE_CONFIG_DIR}else{Join-Path $env:USERPROFILE '.claude'}
Record 'Diretorio personalizado' ([bool]$env:CLAUDE_CONFIG_DIR)
Record 'CLI no PATH deste processo' ([bool](Get-Command claude -ErrorAction SilentlyContinue))
foreach($name in @('ANTHROPIC_API_KEY','ANTHROPIC_AUTH_TOKEN','ANTHROPIC_BASE_URL','CLAUDE_CODE_OAUTH_TOKEN','CLAUDE_CODE_USE_BEDROCK','CLAUDE_CODE_USE_VERTEX','CLAUDE_CODE_USE_FOUNDRY')) {
 Record ($name+' definido') ([bool][Environment]::GetEnvironmentVariable($name))
}
$widget=Join-Path $env:APPDATA 'ClaudeCodeUsageBubble\settings.json'
if(Test-Path -LiteralPath $widget){try{$s=Get-Content -LiteralPath $widget -Raw|ConvertFrom-Json;Record 'Claude habilitado no widget' $s.show_claude_code;Record 'Fonte Claude no widget' (@('auto','subscription','external_api') -contains $s.claude_api.source | ForEach-Object {if($_){$s.claude_api.source}else{'auto/ausente'}})}catch{Record 'Preferencias do widget' 'ilegivel'}}
$settings=Join-Path $root 'settings.json'
if(Test-Path -LiteralPath $settings){try{$s=Get-Content -LiteralPath $settings -Raw|ConvertFrom-Json;foreach($name in @('ANTHROPIC_BASE_URL','ANTHROPIC_API_KEY','ANTHROPIC_AUTH_TOKEN','CLAUDE_CODE_OAUTH_TOKEN')){Record ('settings.env define '+$name) ([bool]$s.env.$name)}}catch{Record 'Configuracao Claude' 'ilegivel'}}
$path=Join-Path $root '.credentials.json'
Record 'Arquivo OAuth encontrado' (Test-Path -LiteralPath $path)
try {
 if(!(Test-Path -LiteralPath $path)){throw 'no_credentials'}
 try{$auth=Get-Content -LiteralPath $path -Raw|ConvertFrom-Json}catch{throw 'invalid_credentials_json'}
 $oauth=$auth.claudeAiOauth
 Record 'Token OAuth presente' ([bool]$oauth.accessToken)
 Record 'Permissao user:profile presente' ([bool](@($oauth.scopes) -contains 'user:profile'))
 if(!$oauth.accessToken){throw 'no_oauth_token'}
 if($oauth.expiresAt){$expired=[DateTimeOffset]::FromUnixTimeMilliseconds([long]$oauth.expiresAt) -lt [DateTimeOffset]::UtcNow;Record 'Token expirado' $expired;if($expired){throw 'expired_token'}}
 # Nao seguir redirecionamentos e nao usar URLs de configuracoes ou gateways.
 $request=[Net.HttpWebRequest]::Create('https://api.anthropic.com/api/oauth/usage')
 $request.Method='GET';$request.AllowAutoRedirect=$false;$request.Timeout=20000
 $request.Headers['Authorization']='Bearer '+$oauth.accessToken
 $request.Headers['anthropic-beta']='oauth-2025-04-20'
 $response=$null
 try{$response=$request.GetResponse()}catch [Net.WebException] {if($_.Exception.Response){$response=$_.Exception.Response}else{throw 'connection_failed'}}
 try {
  $status=[int]$response.StatusCode;Record 'HTTP da consulta oficial' $status
  if($status -ge 200 -and $status -lt 300){
   $reader=New-Object IO.StreamReader($response.GetResponseStream())
   try{$json=$reader.ReadToEnd()|ConvertFrom-Json}finally{$reader.Dispose()}
   foreach($bucket in @('five_hour','seven_day')){
    $value=$json.$bucket
    Record ($bucket+' presente') ([bool]$value)
    Record ($bucket+' percentual numerico') ($null -ne $value.utilization -and $value.utilization -is [ValueType])
    Record ($bucket+' data de renovacao presente') ([bool]$value.resets_at)
   }
  }
 }finally{if($response){$response.Close()}}
}catch{
 $reason=[string]$_.Exception.Message
 if(@('no_credentials','invalid_credentials_json','no_oauth_token','expired_token','connection_failed') -notcontains $reason){$reason='diagnostic_failed'}
 Record 'Resultado' $reason
}finally{
 if($request){$request.Headers.Remove('Authorization')}
 $oauth=$null;$auth=$null
}
$report.Add('Nao inclui chaves, tokens, email, caminhos pessoais ou corpo da resposta. Nao executa login, renovacao ou geracao de texto.')
$report.Add('Se o Claude funciona somente no WSL ou em outro terminal/perfil, informe isso separadamente.')
$output=Join-Path $PSScriptRoot 'diagnostico-claude-pro-sem-segredos.txt'
$report|Set-Content -LiteralPath $output -Encoding UTF8
$report|ForEach-Object{Write-Host $_}
Write-Host 'Relatorio salvo ao lado deste script.'
