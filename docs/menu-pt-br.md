# Menu em português — v0.6.10

## Organização

- **Consultar cota agora:** consulta o consumo da conta; não busca versões do programa.
- **Mostrar widget:** marca ou desmarca a exibição do widget.
- **Exibição e tamanho:** mostrar só sobre a janela do Codex, aumentar, diminuir e restaurar tamanho ou posição.
- **Serviços exibidos:** Codex e Claude Code. OpenCode Go permanece identificado como indisponível.
- **Configurações:** iniciar com o Windows e escolher o intervalo da consulta da cota.
  - Intervalos disponíveis: 1 minuto, 5 minutos, 15 minutos e 1 hora.
  - As notas do submenu explicam que os tokens locais são lidos a cada 5 segundos e o histórico da conta é consultado a cada minuto, independentemente dessa escolha.
- **Atualizações do programa:** buscar e instalar versões automaticamente, pelo menu ou no intervalo escolhido.
  - **Buscar nova atualização (vX.Y.Z)** sempre informa a versão instalada; ao encontrar uma versão nova, baixa, valida e instala sem outro clique.
  - A atualização programada pode ocorrer a cada hora, dia ou semana. A opção **Somente ao buscar pelo menu** desativa a programação; uma busca manual continua instalando a versão encontrada automaticamente.
  - Mensagens de busca, instalação, versão atual e falha ficam separadas dos botões de ação.
- **Como usar:** explica cliques, arraste, tamanho, porcentagem da cota, consumo local e média da conta.
- **Reiniciar widget** e **Sair do widget:** afetam somente este aplicativo.

## Alcance da revisão

Textos de todos os menus e submenus, painel de consumo, dica do ícone perto do relógio, ajuda, avisos de limite, sessão expirada, atualização concluída e recuperação de falha. Nomes de produtos e teclas foram preservados. O formato das preferências e os mecanismos de consulta e instalação foram mantidos.

## Verificação

Os testes constroem e percorrem os menus nativos do Windows nos cinco estados de atualização: inicial, buscando, atualizado, instalando e falha. Conferem o botão único de busca e atualização, IDs sem duplicação, opções marcadas, serviços indisponíveis e limites de tamanho. Esses testes não acionam instalação, saída, reinício nem gravação das preferências reais.

Testes do fluxo automático conferem instalação após detecção, bloqueio de ciclos simultâneos, ausência de instalação quando já atualizado ou quando a consulta falha e liberação para tentar novamente após falha na instalação. O instalador é simulado nesses testes; nenhuma instalação real do usuário é substituída.
