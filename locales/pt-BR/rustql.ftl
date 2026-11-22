# RustQL — português (Brasil).

## Shared

common-cancel = Cancelar
common-error = Erro: { $error }
common-loading = Carregando…
error-title = Erro
toast-close-hint = Clique para fechar
common-close = Fechar
common-delete = Excluir

## Keys (as printed on the keyboard)

key-delete = Del
key-page-up = PgUp
key-page-down = PgDn

## Status bar

status-schema = Esquema
status-log = Log
status-settings = Configurações
status-disconnected = Sem conexão
status-connecting = Conectando a { $host }…

## Settings panel

settings-title = Configurações
settings-language = Idioma
settings-language-system = Sistema
settings-theme = Tema
settings-theme-system = Sistema
settings-theme-light = Claro
settings-theme-dark = Escuro
settings-font = Fonte
settings-font-embedded = Inter (integrada)
settings-font-size = Tamanho da fonte
settings-font-sample = Texto de exemplo
settings-copy-format = Formato de cópia (várias linhas)
settings-page-rows = Linhas por página (tabelas)
settings-editor-max-rows = Máx. de linhas no editor SQL

## Connection tabs

conn-tabs-new = Nova conexão
conn-tabs-close = Fechar conexão

## Connection form

conn-favourites = Favoritos
conn-no-favourites = Nenhuma conexão salva.
conn-no-favourites-hint =
    Preencha o formulário
    e clique em Salvar.
conn-subtitle = Conectar a um banco de dados
conn-engine = Motor:
conn-host = Host:
conn-port = Porta:
conn-user = Usuário:
conn-password = Senha:
conn-password-in-keyring = Salva no chaveiro
conn-database = Banco de dados:
conn-database-all = Todos
conn-database-pick = Escolher entre os bancos de dados do servidor
conn-database-reading = Lendo os bancos de dados…
conn-security = Segurança:
conn-ca-file = Arquivo da CA:
conn-ca-pick = Escolher o arquivo
conn-ca-filter-certificates = Certificados
conn-ca-filter-all = Todos
conn-ca-system = As do sistema
conn-ca-hint =
    Certificado (PEM) da autoridade que assina o do servidor.
    Vazio, confia-se nas autoridades instaladas no sistema.
conn-save-password = Salvar senha
conn-no-keyring = Sem chaveiro do sistema: não será salva
conn-save-password-hint = É salva criptografada no chaveiro do sistema
conn-profile-name = Nome:
conn-profile-name-hint = Meu servidor...
conn-color = Cor:
conn-color-none = Sem cor
conn-color-custom = Personalizada…
conn-save = Salvar
conn-save-as-new = Salvar como novo
conn-save-as-new-hint = Cria outro favorito com estes dados, sem alterar o selecionado
conn-delete = Excluir
conn-connect = Conectar
conn-port-invalid = Porta inválida: “{ $port }”. Deve ser um número entre 1 e 65535.
conn-host-missing = Informe o host do servidor.

## Connection colours

color-blue = Azul
color-teal = Turquesa
color-green = Verde
color-yellow = Amarelo
color-orange = Laranja
color-red = Vermelho
color-pink = Rosa
color-purple = Roxo
color-custom = Personalizada

## Connection security

tls-prefer = Criptografar se possível
tls-require = Exigir criptografia
tls-verify = Verificar o servidor
tls-prefer-description =
    Usa TLS se o servidor oferecer e, se não, conecta sem criptografia.
    Não verifica o certificado.
tls-require-description =
    Não conecta se o servidor não oferecer TLS.
    Não verifica o certificado: protege de quem escuta a rede,
    não de quem se passa pelo servidor.
tls-verify-description =
    Não conecta sem TLS, e verifica se o certificado é desse
    servidor e se foi assinado por uma autoridade em que o sistema confia
    ou pela do arquivo da CA que você indicar.
security-verified = Conexão criptografada (TLS), com o certificado do servidor verificado.
security-encrypted =
    Conexão criptografada (TLS). O certificado do servidor não é verificado:
    para verificá-lo, escolha “{ $verify }” na conexão.
security-local = Conexão sem criptografia, mas ela não sai deste computador.
security-plain =
    Conexão SEM criptografia: tudo o que é enviado e recebido, inclusive a senha,
    pode ser lido na rede. O servidor não oferece TLS.
tls-hint-certificate = Não foi possível verificar o certificado do servidor. Se ele é assinado por uma autoridade própria, indique o arquivo da CA; se não precisa verificá-lo, escolha outro nível de segurança.
tls-hint-required = A segurança desta conexão exige TLS. Se o servidor não oferece, escolha “{ $prefer }”.
tls-ca-unreadable = Não foi possível ler o arquivo da CA “{ $path }”: { $error }
tls-ca-empty = não contém nenhum certificado no formato PEM
tls-verifier-failed = Não foi possível preparar a verificação do certificado: { $error }
tls-setup-failed = Não foi possível preparar o TLS: { $error }

## Connecting

keyring-unavailable = nenhum chaveiro do sistema disponível
keyring-locked = o chaveiro está bloqueado ou o desbloqueio foi cancelado ({ $error })
connect-thread-died = A thread de conexão terminou inesperadamente
connect-starting = Iniciando...
keyring-save-failed = A senha não foi salva no chaveiro: { $error }
connect-reading-keyring = Lendo a senha do chaveiro...
connect-tcp = Abrindo a conexão TCP...
connect-authenticating = Autenticando...
connect-connecting = Conectando ao servidor...
connect-established = Conexão estabelecida.
connect-setup-failed = Não foi possível preparar a conexão: { $error }
connect-failed = Não foi possível conectar: { $error }{ $hint }
connect-hint-database = Se o servidor só deixa você entrar no seu próprio banco de dados, informe-o no campo “Banco de dados”.
keyring-read-failed = Não foi possível ler a senha do chaveiro: { $error }
keyring-password-gone = A senha deste perfil não está mais no chaveiro do sistema: digite-a e salve o perfil de novo.

## Session

session-connecting = Conectando a { $host }...
session-pick-database = Selecione um banco de dados no painel à esquerda
session-pick-table = Selecione uma tabela no painel à esquerda

## Session tab bar

tab-database = Banco
tab-structure = Estrutura
tab-structure-of = Estrutura de { $table }
tab-data = Dados
tab-rename = Renomear
tab-close = Fechar
tab-close-sql-hint = Fechar a aba SQL  { $shortcut } (ou clique do meio na aba)
tab-new-sql = Nova aba SQL
tab-name-hint = Nome da aba

## Schema tree

tree-title = Esquema
tree-showing-favourites = Mostrando só favoritos
tree-filter-favourites = Filtrar favoritos
tree-reload = Recarregar esquema
tree-search = Buscar…
tree-create-database = Criar banco de dados
tree-refresh = Atualizar
tree-disconnect = Desconectar
tree-no-matches = (nenhum resultado)
tree-no-tables = (nenhuma tabela)
tree-create-table = Criar tabela
tree-export-database = Exportar banco de dados
tree-empty-all-tables = Esvaziar todas as tabelas
tree-drop-database = Excluir banco de dados
tree-unfavourite = Remover dos favoritos
tree-favourite = Marcar como favorito
tree-show-structure = Ver estrutura
tree-open-in-sql = Abrir em SQL
tree-copy-name = Copiar nome
tree-show-definition = Ver definição
tree-export-view = Exportar visão
tree-drop-view = Excluir visão
tree-clone-table = Clonar tabela
tree-export-table = Exportar tabela
tree-empty-table = Esvaziar tabela
tree-drop-table = Excluir tabela

## Server and database panels

server-info-title = Servidor: { $host }
server-info-database-count = { $count ->
    [one] — { $count } banco de dados
   *[other] — { $count } bancos de dados
}
server-info-no-databases = (nenhum banco de dados)
server-info-col-database = Banco de dados
server-info-col-tables = Tabelas
server-info-col-size = Tamanho
db-info-title = Banco de dados: { $db }
db-info-table-count = { $count ->
    [one] — { $count } tabela
   *[other] — { $count } tabelas
}
db-info-no-tables = Este banco de dados não tem tabelas.
db-info-col-table = Tabela
db-info-col-engine = Motor
db-info-col-rows = Linhas
db-info-col-size = Tamanho
db-info-open-hint = Clique duas vezes para abrir a tabela

## Table structure

structure-column-count = { $count ->
    [one] — { $count } coluna
   *[other] — { $count } colunas
}
structure-no-columns = (nenhuma coluna)
structure-col-column = Coluna
structure-col-type = Tipo
structure-col-key = Chave

## Data grid

grid-sort-asc-hint = Clique: ordenar ↑
grid-sort-desc-hint = Clique: ordenar ↓
grid-sort-off-hint = Clique: remover ordenação
grid-remove-new-row = Remover a linha nova
grid-running-query = Executando consulta…
grid-page-empty = — nenhuma linha nesta página
grid-page-rows = — linhas { $first }–{ $last }
grid-page-size-hint = São pedidas { $count } linhas por página (altere em Configurações).
grid-page-previous = Anterior
grid-page-previous-hint = Página anterior
grid-page-next = Próxima
grid-page-next-hint = Próxima página
grid-row-count = { $count ->
    [one] — { $count } linha
   *[other] — { $count } linhas
}
grid-view-read-only = Visão · somente leitura
grid-view-read-only-hint =
    Os dados de uma visão são os das suas tabelas: edite-os nelas.
    A sua definição é aberta pelo menu da visão na árvore.
grid-row = Linha
grid-insert-row-hint = Inserir uma linha
grid-no-primary-key = Sem chave primária
grid-no-primary-key-hint =
    Sem chave primária não há um jeito seguro de identificar uma linha:
    as linhas existentes não podem ser editadas nem excluídas daqui.
    Ainda é possível adicionar linhas novas.
grid-sort = Ordem:
grid-sort-remove = Remover “{ $column }”
grid-sort-clear = Limpar tudo
grid-sort-clear-hint = Remover toda a ordenação

## Grid context menu

menu-copy-value = Copiar valor
menu-copy-csv = Copiar como CSV
menu-copy-tsv = Copiar como TSV
menu-copy-inserts = Copiar como SQL INSERT
menu-edit-cell = Editar célula
menu-set-null = Definir NULL
menu-set-empty = Definir vazio
menu-now = Data/hora atual
menu-default-value = Valor padrão
menu-undo-change = Desfazer a alteração
menu-filter-clipboard = Pela área de transferência
menu-filter-text = Por texto...
menu-insert-row = Inserir linha
menu-clone-row = Clonar linha
menu-undelete = Desmarcar exclusão
menu-delete-rows = { $count ->
    [one] Excluir linha
   *[other] Excluir { $count } linhas
}

## Filter

filter-history-hint = Filtros recentes
filter-no-history = Sem histórico
filter-hint = id = 1 AND nome LIKE 'foo%'
filter-field-hint = SQL direto — adicionado à consulta como cláusula WHERE
filter-apply = Aplicar
filter-clear = Limpar filtro
filter-by-title = Filtrar por { $column }
filter-operator = Operador:
filter-value = Valor:
filter-value-hint = valor a filtrar
filter-op-equals = igual a
filter-op-not-equals = diferente de
filter-op-contains = contém
filter-op-starts-with = começa com
filter-op-ends-with = termina com
filter-op-greater = maior que
filter-op-less = menor que
filter-op-greater-or-equal = maior ou igual
filter-op-less-or-equal = menor ou igual

## Pending changes and saving

apply-no-primary-key = Sem chave primária — não é possível salvar
apply-edits = { $count ->
    [one] { $count } edição
   *[other] { $count } edições
}
apply-deletes = { $count ->
    [one] { $count } exclusão
   *[other] { $count } exclusões
}
apply-inserts = { $count ->
    [one] { $count } inserção
   *[other] { $count } inserções
}
apply-discard = Descartar
apply-save = Salvar
save-no-open-table = Nenhuma tabela aberta.
save-no-data = Nenhum dado carregado.
save-structure-loading = A estrutura da tabela ainda está sendo lida: tente de novo.
save-no-primary-key = A tabela não tem chave primária: não é possível editar nem excluir linhas.
save-no-column = A coluna { $index } não existe.
save-cannot-title = Não é possível salvar as alterações
save-failed-title = Erro ao salvar
log-save-changes = Salvar alterações: { $deletes } exclusão(ões), { $edits } edição(ões), { $inserts } inserção(ões) (transação)
changes-saved = { $count ->
    [one] { $count } alteração salva em “{ $table }”
   *[other] { $count } alterações salvas em “{ $table }”
}
save-row-key-missing = Não foi encontrada a chave primária da linha { $row }.
save-row-not-found = Nada foi salvo: uma instrução devia alterar uma linha e encontrou { $found }. A linha pode ter mudado ou sumido no servidor; recarregue a tabela (F5) e refaça as alterações.

## SQL editor

sql-tab-title = SQL { $number }
sql-database = BD:
sql-no-database = (nenhum)
sql-free-query = SQL livre:
sql-run-selection = Executar seleção
sql-run = Executar
sql-clear = Limpar
sql-cancelling = Cancelando…
sql-running = Executando…
sql-cancel-hint = Interrompe a consulta no servidor
sql-editor-hint = SELECT * FROM tabela LIMIT 100
sql-empty = Escreva uma consulta e pressione { $shortcut } ou o botão Executar
sql-cancelled = Consulta cancelada
sql-done-no-result = Executada, sem resultado para mostrar
sql-result-summary = { $rows ->
    [one] { $rows } linha
   *[other] { $rows } linhas
}, { $columns ->
    [one] { $columns } coluna
   *[other] { $columns } colunas
}
sql-result-truncated = só as primeiras { $count } (há mais; o limite é alterado em Configurações)

## Query log

log-title = Log de consultas
log-entry-count = { $count ->
    [one] { $count } entrada
   *[other] { $count } entradas
}
log-cancelled = CANCELADA
log-rows = { $count ->
    [one] { $count } linha
   *[other] { $count } linhas
}
log-copy-query = Copiar a consulta
log-copy-error = Copiar o erro
log-open-in-sql-tab = Abrir em uma aba SQL
log-error = ERRO
log-cancel-query = Cancelar a consulta
log-export = Exportação
log-export-finished = { $count ->
    [one] Exportação concluída: { $count } tabela
   *[other] Exportação concluída: { $count } tabelas
}

## Create, drop and empty dialogs

discard-sql-close-title = Fechar “{ $title }”
discard-sql-close = Fechar aba
discard-sql-clear-title = Limpar “{ $title }”
discard-sql-warning = A consulta desta aba será perdida:
create = Criar
create-name = Nome:
create-database-name-hint = meu_banco
create-table-name = Nome da tabela:
create-table-name-hint = minha_tabela
create-columns = Colunas:
create-col-name = Nome
create-col-length = Tamanho
create-add-column = Adicionar coluna
drop-confirm-database = Tem certeza de que quer excluir o banco de dados:
drop-confirm-view = Tem certeza de que quer excluir a visão:
drop-confirm-table = Tem certeza de que quer excluir a tabela:
drop-cannot-undo = Esta ação não pode ser desfeita.
drop-type-name = Digite o nome para confirmar:
truncate-title = Esvaziar dados
truncate-confirm-all = Tem certeza de que quer esvaziar todas as tabelas de:
truncate-confirm-table = Tem certeza de que quer esvaziar a tabela:
truncate-warning = Todos os dados serão excluídos. Não é possível desfazer.
truncate-button = Esvaziar
unsaved-title = Alterações não salvas
unsaved-changes = { $count ->
    [one] Você tem { $count } alteração não salva em “{ $table }”.
   *[other] Você tem { $count } alterações não salvas em “{ $table }”.
}
unsaved-switch-question = O que você quer fazer antes de abrir “{ $table }”?
unsaved-save-and-switch = Salvar e trocar
unsaved-discard-and-switch = Descartar e trocar
unsaved-lost-on-page = Ao mudar de página elas seriam perdidas.
unsaved-lost-on-reload = Ao recarregar os dados elas seriam perdidas.
unsaved-save-and-reload = Salvar e recarregar
unsaved-discard-and-reload = Descartar e recarregar
unsaved-lost-on-close = Se você fechar a aba, elas serão perdidas.
unsaved-discard-and-close = Descartar e fechar
clone-title = Clonar tabela — { $table }
clone-new-name = Novo nome:
clone-name-hint = { $table }_copia
clone-copy-data = Copiar dados:
clone-button = Clonar

## Favourites

favourite-deleted = Favorito “{ $name }” excluído
favourite-saved = Favorito “{ $name }” salvo
favourite-name-missing = Dê um nome ao favorito para salvá-lo.
favourite-name-taken = Já existe um favorito chamado “{ $name }”: mude o nome para salvá-lo como outro.
favourite-delete-title = Excluir favorito
favourite-delete-question = Tem certeza de que quer excluir o favorito:
favourite-delete-keyring = A senha dele também será removida do chaveiro.

## Notifications

database-list-failed = Não foi possível ler a lista de bancos de dados.
clipboard-read-failed = Não foi possível ler a área de transferência: { $error }
name-copied = “{ $name }” copiado para a área de transferência
rows-copied = { $count ->
    [one] { $count } linha copiada ({ $format })
   *[other] { $count } linhas copiadas ({ $format })
}
database-created = Banco de dados “{ $db }” criado
database-dropped = Banco de dados “{ $db }” excluído
database-drop-failed = Não foi possível excluir “{ $db }”
table-created = Tabela “{ $table }” criada
table-dropped = “{ $table }” excluída
table-drop-failed = Não foi possível excluir “{ $table }”
tables-list-failed = Não foi possível listar as tabelas de “{ $db }”
tables-empty-failed = Não foi possível esvaziar todas as tabelas de “{ $db }”
table-emptied = Tabela “{ $table }” esvaziada
tables-emptied = { $count ->
    [one] { $count } tabela de “{ $db }” esvaziada
   *[other] { $count } tabelas de “{ $db }” esvaziadas
}

## Export dialog

export-title-table = Exportar “{ $table }”
export-thread-died = A thread de exportação terminou inesperadamente.
export-title = Exportar dados — { $db }
export-tables-heading = Tabelas ({ $selected }/{ $total })
export-select-all = Todas
export-select-none = Nenhuma
export-destination = Destino
export-destination-label = Destino:
export-to-file = Arquivo SQL (.sql)
export-to-folder = Pasta (um .sql por tabela)
export-to-database = Outro banco de dados (mesmo servidor)
export-to-server = Outro servidor (favoritos)
export-choose = Selecionar…
export-no-profiles = Sem perfis
export-no-profiles-hint = Salve antes um perfil com o servidor de destino.
export-target-database = Banco de destino:
export-options = Opções
export-include-create = Incluir CREATE TABLE
export-include-data = Incluir dados (INSERT INTO)
export-button = Exportar
export-select-a-table = Selecione pelo menos uma tabela
export-progress = { $done }/{ $total } tabelas
export-tables-done = Tabelas concluídas: { $done } / { $total }
export-rows-done = Linhas exportadas:   { $rows }
export-rows-unit = linhas
export-cancel = Cancelar exportação
export-completed = Concluído
export-completed-errors = { $count ->
    [one] Concluído com { $count } erro
   *[other] Concluído com { $count } erros
}
export-done = Exportação concluída
export-done-errors = Exportação concluída com erros
export-summary-tables = Tabelas: { $count }
export-summary-errors = Com erro: { $count } (detalhes no log)
export-summary-rows = Linhas:  { $rows }
export-summary-time = Tempo: { $seconds } s
export-cancelled = Cancelado
export-failed = A exportação falhou:
export-same-target = O destino é o mesmo banco de dados do mesmo servidor que a origem: a cópia apagaria ou duplicaria os seus dados. Escolha outro banco de destino.
export-source-connection = Conexão com a origem: { $error }
export-target-connection = Conexão com o destino: { $error }
export-same-server-check-failed = Não foi possível verificar se o destino é o mesmo servidor: { $error }
export-create-target-failed = Não foi possível criar o banco de destino “{ $db }”: { $error }
export-file-create-failed = Não foi possível criar o arquivo “{ $path }”: { $error }
export-file-write-failed = Erro ao escrever o arquivo: { $error }
export-folder-create-failed = Não foi possível criar a pasta “{ $folder }”: { $error }
export-cancelled-error = Exportação cancelada.
export-drop-existing-failed =
    Não foi possível excluir do destino as tabelas que já existem: { $error }

    Se outras tabelas ou visões do destino dependem delas, exporte-as também ou exclua-as antes.
export-no-ddl = Não foi obtido DDL para a tabela “{ $table }”
export-table-gone = A tabela “{ $table }” não existe mais.
export-foreign-table = As tabelas estrangeiras não são exportadas.
export-dump-header = Exportação RustQL — Banco de dados: { $db }
export-dump-generated = Gerado em: { $date }
export-dump-table = Tabela: { $table }
export-dump-deferred = Visões e chaves estrangeiras
export-partitioned-table = As tabelas particionadas não são exportadas: as suas partições não seriam copiadas. Use o pg_dump.
export-postgres-scope = São copiadas tabelas e visões com as suas sequências, chaves e índices. Não gatilhos, funções, permissões nem partições: para uma cópia completa, use o pg_dump.

## Database errors

view-definition-failed = Não foi possível ler a definição de “{ $view }”: { $error }
cancel-query-failed = Não foi possível cancelar a consulta: { $error }
db-no-connection = Sem conexão com o servidor: { $error }
db-no-connection-to = Sem conexão com “{ $db }”: { $error }
db-query-error = Erro na consulta: { $error }
db-write-error = Erro de escrita: { $error }
db-begin-error = Não foi possível iniciar a transação: { $error }
db-transaction-error = Erro na transação (ROLLBACK): { $error }
db-commit-error = Erro no COMMIT: { $error }
editor-session-failed = Não foi possível abrir a sessão do editor: { $error }
view-definition-missing = Não foi encontrada a definição da visão “{ $view }”.
view-materialized-comment =
    -- Uma visão materializada não pode ser substituída: para alterá-la
    -- é preciso excluí-la e criá-la de novo.
charset-invalid = Charset inválido: “{ $charset }”. Só são permitidos caracteres alfanuméricos e “_”.
