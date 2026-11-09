# CODEX + CLAUDE Widget para Windows

Widget flutuante com duas bolhas identificadas: **CODEX** e **CLAUDE**.
Cada pessoa consulta suas próprias contas, autenticadas pelos CLIs oficiais.

## Instalação fácil

1. [Baixe o ZIP oficial](https://github.com/rafaelcaus/codex-usage-bubble/releases/latest/download/Codex-Claude-Widget-Windows.zip).
2. Extraia a pasta inteira e abra **Instalar Widget.cmd**.
3. O instalador verifica SHA-256, instala somente no perfil do usuário e cria um atalho na área de trabalho. Não precisa de administrador nem de login no GitHub.
4. Instale os CLIs oficiais dos serviços que pretende consultar e faça seu próprio login. O instalador não instala CLIs nem solicita senhas.

Codex e Claude ficam habilitados. Se uma conta não estiver conectada, seus dados podem aparecer indisponíveis; isso não impede o outro serviço de funcionar.

## Escolher o que aparece

Botão direito na bolha → **Abrir configurações → Informações**:
- Quando renova.
- Segundo limite, quando fornecido pelo serviço.
- Uso de hoje em tokens do Codex.
- Média de 7 dias do Codex.
- Resets disponíveis do Codex.

As opções valem para as duas bolhas e ficam salvas. Média da conta e resets são recursos do Codex; valores não suportados pelo Claude não são inventados. O cartão se ajusta ao conteúdo. Modo compacto mantém o nome e o anel.

## Atualizações automáticas

O atalho **Atualizar aplicativo** fica diretamente no menu do botão direito e procura/instala uma versão mais nova. A versão instalada aparece no submenu de atualizações.

Ativas a cada hora pelo instalador. O aplicativo procura somente releases deste fork e valida o SHA-256 informado pelo GitHub antes de substituir o executável. Configurações e histórico são preservados. Botão direito permite conferir atualizações ou mudar a frequência.

## Dados e segurança

Leia [SECURITY.md](SECURITY.md). O aplicativo usa credenciais já existentes dos CLIs no próprio computador. Não distribui contas do autor, tokens GitHub ou senhas. Configurações e histórico ficam em `%APPDATA%\ClaudeCodeUsageBubble`; o executável fica em `%LOCALAPPDATA%\ClaudeCodeUsageBubble`.

Histórico Codex com 35 dias de retenção: veja [HISTORY.txt](HISTORY.txt). Registra percentual, tokens e mudanças de ciclo enquanto o aplicativo está aberto. Não é uma franquia fixa em tokens.

## Desenvolvimento

Rust/Win32, Windows x64 e ferramentas MSVC/Windows SDK.

```powershell
cargo test --release --locked
cargo build --release --locked
.\package.ps1
```

O pacote sai em `dist/Codex-Claude-Widget-Windows.zip`. O empacotador usa uma lista fechada de arquivos; não inclui dados do perfil, credenciais, `.git`, backups ou logs.

Fork: [rafaelcaus/codex-usage-bubble](https://github.com/rafaelcaus/codex-usage-bubble).
Derivado de [tiennm99/claude-code-usage-bubble](https://github.com/tiennm99/claude-code-usage-bubble), licença Apache-2.0. O nome interno do executável continua `claude-code-usage-bubble.exe` por compatibilidade.

## Assinatura, API e gateway são fontes diferentes

Em **Abrir configurações → Contas e APIs**, selecione CODEX ou CLAUDE e a fonte: Automático, Assinatura do serviço ou API externa. O modo automático consulta a configuração local e sinaliza gateways; não usa a cota Free como se fosse o saldo da API. Uma assinatura e uma API podem coexistir e ter limites diferentes.

Para API externa, informe uma URL HTTPS **GET de consumo**, autenticação e os caminhos JSON de consumo usado e limite total do mesmo período (por exemplo `/quota/used` e `/quota/limit`). A URL de geração de respostas normalmente não serve. Data de renovação e período são opcionais; o widget não inventa esses valores. A chave é guardada no Gerenciador de Credenciais do usuário Windows, vinculada ao serviço e à URL exata. Tokens da assinatura nunca são reutilizados no gateway; redirecionamentos da consulta externa são bloqueados.

A Clipoos documenta a consulta `https://clipoos.online/usage`. O formato da resposta autenticada ainda precisa ser confirmado para fornecer preenchimento automático; a integração genérica permite campos explícitos. Não use POST nesse endereço para monitorar consumo. Nenhuma chave deve ser enviada por chat.

A nova janela reúne Geral, Informações e Contas e APIs. **Salvar alterações** aplica as escolhas; **Cancelar** descarta a edição. O menu curto mantém atualizar dados, detalhes, modo compacto e atualizar aplicativo. Recursos antigos continuam em Mais opções.

O Claude usa as credenciais locais do Claude Code, incluindo perfis com `CLAUDE_CONFIG_DIR`. Login no site/aplicativo não substitui login no Claude Code. Falhas de autenticação do computador de outra pessoa exigem diagnóstico local; não são resolvidas copiando credenciais do autor.
Na v0.6.19, instalações anteriores recebem uma migração única que ativa a checagem automática a cada hora, inclusive se antes estava diária ou desativada. Depois disso o usuário pode escolher outra frequência nas configurações. A versão antiga precisa primeiro receber essa atualização; se estava no modo manual, use Atualizar aplicativo uma vez. O widget precisa estar aberto e conectado à internet.
