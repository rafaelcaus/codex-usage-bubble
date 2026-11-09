# Segurança e distribuição

## O que compartilhar

Compartilhe apenas o link público deste repositório ou o ZIP da release. Baixar um repositório público dá acesso de leitura ao código, não permissão de escrita no GitHub nem acesso ao computador do autor. Não compartilhe sua pasta de perfil ou um ZIP manual do AppData.

O pacote é montado por lista fechada: executável, instaladores, documentação, licença e hashes. Não inclui autenticação, histórico, preferências pessoais, repositório .git, backups ou logs.

## Autenticação e acesso local

O widget lê as credenciais dos CLIs oficiais na máquina da pessoa que o executa. Essas credenciais são sensíveis: não envie arquivos `.credentials.json`, `auth.json`, `.env` nem tokens para outra pessoa. O programa não precisa de GitHub CLI ou token GitHub para instalar, rodar ou atualizar.

A instalação é por usuário, em pastas específicas AppData. O processo usa as permissões normais da conta Windows: não existe sandbox que impeça qualquer software executado pelo usuário de acessar outras pastas permitidas. Não há garantia absoluta contra invasões. O widget não cria acesso remoto, servidor de edição nem compartilhamento de arquivos.

## Atualização

Somente o asset exato `claude-code-usage-bubble.exe` no repositório `rafaelcaus/codex-usage-bubble` é aceito. Atualizações sem digest SHA-256 são rejeitadas. O hash é conferido antes da troca do arquivo. Isso detecta divergências; não é assinatura independente nem proteção caso a conta mantenedora e a release sejam comprometidas juntas.

O ZIP local tem hashes para verificação de integridade. Se alguém adulterar todo o ZIP junto com os hashes, essa verificação sozinha não prova autoria. Use o endereço oficial de download.

## Administração do GitHub

Código público pode ser copiado e receber propostas por fork/PR, conforme a licença. Alterar a versão oficial exige permissão de escrita. Proteja a conta mantenedora com autenticação forte e revise colaboradores, chaves e permissões. O instalador não altera essas configurações.

Novas releases precisam passar pelos testes. Workflow de release roda por tags ou ação manual, sem executar conteúdo de PR de desconhecidos com token de escrita.

## APIs externas

A chave informada localmente fica no Gerenciador de Credenciais do usuário Windows, vinculada ao provedor e ao hash da URL exata. Ela não entra em settings.json nem no pacote. O widget não reutiliza tokens OAuth das assinaturas para autenticar gateways. A consulta é somente GET sobre HTTPS, sem redirecionamentos, URL com senha ou chave em parâmetros. O usuário deve confiar no endereço configurado: é para esse fornecedor que a chave será enviada. Outros programas executados como o mesmo usuário podem ter acesso às credenciais; isso não é isolamento contra software malicioso local.

O fornecedor precisa expor consumo e limite do mesmo período. Campos ausentes, limite zero ou respostas inválidas não viram uma cota presumida. O aplicativo não chama geração de texto nem endpoints POST de reset para testar a integração.
