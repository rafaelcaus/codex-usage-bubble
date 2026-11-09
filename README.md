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

Botão direito na bolha → **Configurações → Informações no widget**:
- Quando renova.
- Segundo limite, quando fornecido pelo serviço.
- Uso de hoje em tokens do Codex.
- Média de 7 dias do Codex.
- Resets disponíveis do Codex.

As opções valem para as duas bolhas e ficam salvas. Média da conta e resets são recursos do Codex; valores não suportados pelo Claude não são inventados. O cartão se ajusta ao conteúdo. Modo compacto mantém o nome e o anel.

## Atualizações automáticas

Ativas diariamente pelo instalador. O aplicativo procura somente releases deste fork e valida o SHA-256 informado pelo GitHub antes de substituir o executável. Configurações e histórico são preservados. Botão direito permite conferir atualizações ou mudar a frequência.

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
