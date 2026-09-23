# Fork — Codex semanal (Pro)

Fork pessoal de
[tiennm99/claude-code-usage-bubble](https://github.com/tiennm99/claude-code-usage-bubble)
(Apache-2.0). Otimizado para quem usa **Codex no plano Pro (cota semanal)**
pelo app desktop ChatGPT.

## O que mudou em relacao ao upstream (v0.6.0)

- Bolha vertical minimalista, 100% proporcional do tamanho 50 ao 360
  (`Ctrl + rodinha` para redimensionar, min 50).
- Anel = **% restante da cota semanal** (ex.: `RESTA 97%`), com countdown
  preciso abaixo (`6d 19h`; abaixo de 1 dia mostra horas+minutos).
- Janela de 5h removida de todas as superficies (anel, painel, bandeja,
  alertas) — no Pro, a janela primaria JA E a cota semanal.
- Painel expandido: barra `7d` (`usada/resta`), `Falta para resetar: Xd Xh`,
  `Tokens neste PC desde o reset: N` (somado dos rollouts locais
  `~/.codex/sessions`, somente leitura; reflete uso via CLI neste PC).
- Menu Settings: **"Somente sobre o ChatGPT"** — a bolha se esconde quando
  outro app esta em foco (cheque a cada ~0,35s, com anti-flicker) e volta
  com o ChatGPT. Desligue para flutuar sobre tudo.
- Clique sem arrastar alterna o painel (tolerancia de 8px para micro-movimento).
- Self-update aponta para **este repositorio** (`rafaelcaus/codex-usage-bubble`);
  padrao de checagem: Daily. Releases aqui atualizam todos os PCs sozinhas.
- Textos novos em PT-BR no painel/tooltip (`usada/resta`).

## Instalacao em um PC novo (Windows 10/11)

1. Instale o Codex CLI oficial e faca login (a bolha le o consumo via
   autenticacao local do Codex):
   - `npm install -g @openai/codex` (ou o instalador oficial)
   - `codex login status` deve indicar login valido.
2. Baixe o `claude-code-usage-bubble.exe` da
   [latest release](../../releases/latest) e coloque em
   `%LOCALAPPDATA%\ClaudeCodeUsageBubble\`.
3. Execute uma vez. Na primeira vez o SmartScreen avisa (binario sem
   assinatura): **Mais informacoes → Executar assim mesmo**.
4. Botao direito na bolha → Providers: deixe so **Codex** ativo.
5. Opcional: Settings → **Start with Windows** para abrir junto com o PC.
6. Pronto. Com o auto-update em Daily, as proximas versoes deste fork
   chegam sozinhas.

## Publicando uma nova versao (este PC, dev)

1. Edite o codigo em `src/`.
2. Suba a `version` em `Cargo.toml` (ex.: `0.6.1`).
3. `cargo build --release` (+ `cargo test --release`).
4. Commit + push + tag: `git tag v0.6.1 && git push origin v0.6.1`.
5. `gh release create v0.6.1 target/release/claude-code-usage-bubble.exe --title v0.6.1 --generate-notes`
6. Todos os PCs com o fork atualizam sozinhos no proximo cheque Daily
   (ou via Settings → Check for updates).

## Notas

- `auth.json` nunca e lido na integra, copiado ou modificado pelo app —
  apenas as credenciais locais ja existentes sao usadas para consultar o uso.
- `settings.json` (`%APPDATA%\ClaudeCodeUsageBubble`) nunca e apagado por
  updates; so o `.exe` e substituido (com backup automatico do updater).
