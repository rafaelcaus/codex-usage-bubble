# Instalar CODEX + CLAUDE

Baixe o pacote Windows na [release oficial](https://github.com/rafaelcaus/codex-usage-bubble/releases/latest).
Extraia todos os arquivos do ZIP e dê duplo clique em **Instalar Widget.cmd**.
Não execute de dentro da visualização compactada do ZIP.

Não precisa de administrador, token GitHub nem senha do autor. Cada pessoa precisa instalar e autenticar os CLIs oficiais que usa: Codex e/ou Claude Code. Sem login, o respectivo consumo ficará indisponível. Nenhum login é feito automaticamente pelo instalador.

As duas bolhas são ativadas e identificadas no topo. Atualizações automáticas a cada hora ficam habilitadas.

Botão direito → Abrir configurações → Informações para escolher os dados visíveis.
Abrir configurações → Geral permite ajustar tamanho, modo compacto e a opção de aparecer só sobre ChatGPT. Em instalação nova as bolhas aparecem sobre todos os aplicativos, para também permitir uso com Claude.

SHA-256 acompanha o ZIP em SHA256SUMS.txt. O instalador valida o executável antes de modificar a instalação. Ele preserva configurações e histórico e guarda backup do executável anterior.

Leia SECURITY.md para o escopo e as limitações dessa verificação. Binários sem assinatura podem receber aviso do Windows; confira origem e integridade antes de decidir executar. Não desative o antivírus nem as proteções do Windows.

Para APIs e gateways, veja Contas e APIs: é necessária uma consulta de consumo do fornecedor, além da URL usada para gerar respostas. A chave fica no Windows de cada usuário; não compartilhe credenciais.

Na v0.6.20, instalações anteriores recebem uma migração única que ativa a checagem automática a cada hora, inclusive se antes estava diária ou desativada. Depois disso o usuário pode escolher outra frequência nas configurações. A versão antiga precisa primeiro receber essa atualização; se estava no modo manual, use Atualizar aplicativo uma vez. O widget precisa estar aberto e conectado à internet.
