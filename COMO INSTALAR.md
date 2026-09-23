# Instalar este fork em outro PC (guia para voce ou para o seu agente)

> Para o **OpenCode/agent em outro PC**: leia este arquivo e execute
> `install.ps1` — ele faz a instalacao completa sozinho e imprime o
> relatorio de validacao no final. so peca ajuda manual para: login do
> Codex (interativo) e o clique do SmartScreen (grafico do Windows).

Repo: https://github.com/rafaelcaus/codex-usage-bubble
Script: https://raw.githubusercontent.com/rafaelcaus/codex-usage-bubble/main/install.ps1

## Pre-requisitos (o script confere e ajuda)

- Windows 10/11.
- Node LTS + npm (https://nodejs.org). O script instala o Codex CLI via
  `npm install -g @openai/codex` se ele estiver ausente.
- Conta com o app desktop ChatGPT/Codex (plano com cota) — o login do
  Codex precisa existir **neste PC** (cada PC loga o seu; nunca copie
  `auth.json` entre maquinas).

## Instalacao automatica (recomendado)

No PowerShell do outro PC:

```powershell
irm https://raw.githubusercontent.com/rafaelcaus/codex-usage-bubble/main/install.ps1 | iex
```

O script, sem precisar de admin:

1. Confere `codex --version` e mostra o status do login (se nao estiver
   logado, rode `codex login` manualmente e rode o script de novo).
2. Pega a latest release **deste repositorio** (nunca de outro lugar).
3. Valida **tamanho + SHA-256** contra a API do GitHub antes de instalar.
4. Instala em `%LOCALAPPDATA%\ClaudeCodeUsageBubble\`
   (com backup automatico do `.exe` anterior, se houver).
5. Configura **Codex=ON, Claude=OFF** (com backup do `settings.json`).
6. Registra **uma** entrada de "Start with Windows" (mecanismo nativo).
7. Inicia a bolha e valida: processo no ar, startup registrada.
8. Mantem padroes do fork: poll 1 min, tamanho 50, `Somente sobre o
   ChatGPT` ligado, auto-update **Daily** (do proprio repo).

## Intervencoes manuais (unicas possiveis)

1. **SmartScreen** (binario sem assinatura, esperado): na primeira
   execucao clique **Mais informacoes → Executar assim mesmo**.
2. **`codex login`**: se o script avisar que nao ha login valido.

## Pos-instalacao (2 min)

- Botao direito na bolha → Providers: confira que so **Codex** esta ativo.
- Arraste a bolha para a lateral (a posicao salva sozinha).
- Clique seco nela: painel com `7d`, `usada/resta`, `Falta para resetar`
  e tokens. `Ctrl + rodinha` em cima dela muda o tamanho (50–360).

## Como atualizar depois

Nao precisa fazer nada: o auto-update Daily baixa as novas releases
**deste repositorio** sozinho (com verificacao SHA-256). Ou use
Settings → Check for updates no menu da bolha.

## Solucao de problemas

- Bolha sem dados do Codex: `codex --version`, `codex login status`,
  confira `%USERPROFILE%\.codex\auth.json` (so existencia) e se Codex
  esta ativo no menu Providers.
- Log de diagnostico: rode o exe com `--diagnose` uma vez e leia
  `%TEMP%\claude-code-usage-bubble.log` (sem credenciais).
- Detalhes do fork e changelog: ver [FORK.md](FORK.md).
