# RustQL — italiano.

## Shared

common-cancel = Annulla
common-error = Errore: { $error }
common-loading = Caricamento…
error-title = Errore
toast-close-hint = Clic per chiudere
common-close = Chiudi
common-delete = Elimina

## Keys (as printed on the keyboard)

key-delete = Canc
key-page-up = PagSu
key-page-down = PagGiù

## Status bar

status-schema = Schema
status-log = Registro
status-settings = Impostazioni
status-disconnected = Non connesso
status-connecting = Connessione a { $host }…

## Settings panel

settings-title = Impostazioni
settings-language = Lingua
settings-language-system = Sistema
settings-theme = Tema
settings-theme-system = Sistema
settings-theme-light = Chiaro
settings-theme-dark = Scuro
settings-font = Carattere
settings-font-embedded = Inter (integrato)
settings-font-size = Dimensione del carattere
settings-font-sample = Testo di esempio
settings-copy-format = Formato di copia (più righe)
settings-page-rows = Righe per pagina (tabelle)
settings-editor-max-rows = Righe max. nell’editor SQL

## Connection tabs

conn-tabs-new = Nuova connessione
conn-tabs-close = Chiudi la connessione

## Connection form

conn-favourites = Preferiti
conn-no-favourites = Nessuna connessione salvata.
conn-no-favourites-hint =
    Compila il modulo
    e premi Salva.
conn-subtitle = Connettiti a un database
conn-engine = Motore:
conn-host = Host:
conn-port = Porta:
conn-user = Utente:
conn-password = Password:
conn-password-in-keyring = Salvata nel portachiavi
conn-database = Database:
conn-database-all = Tutti
conn-database-pick = Scegli tra i database del server
conn-database-reading = Lettura dei database…
conn-security = Sicurezza:
conn-ca-file = File della CA:
conn-ca-pick = Scegli il file
conn-ca-filter-certificates = Certificati
conn-ca-filter-all = Tutti
conn-ca-system = Quelle del sistema
conn-ca-hint =
    Certificato (PEM) dell’autorità che firma quello del server.
    Se vuoto, si usano le autorità installate nel sistema.
conn-save-password = Salva la password
conn-no-keyring = Nessun portachiavi di sistema: non verrà salvata
conn-save-password-hint = Viene salvata cifrata nel portachiavi di sistema
conn-profile-name = Nome:
conn-profile-name-hint = Il mio server...
conn-color = Colore:
conn-color-none = Nessun colore
conn-color-custom = Personalizzato…
conn-save = Salva
conn-save-as-new = Salva come nuovo
conn-save-as-new-hint = Crea un altro preferito con questi dati, senza toccare quello selezionato
conn-delete = Elimina
conn-connect = Connetti
conn-port-invalid = Porta non valida: «{ $port }». Deve essere un numero tra 1 e 65535.
conn-host-missing = Indica l’host del server.

## Connection colours

color-blue = Blu
color-teal = Turchese
color-green = Verde
color-yellow = Giallo
color-orange = Arancione
color-red = Rosso
color-pink = Rosa
color-purple = Viola
color-custom = Personalizzato

## Connection security

tls-prefer = Cifra se possibile
tls-require = Richiedi la cifratura
tls-verify = Verifica il server
tls-prefer-description =
    Usa TLS se il server lo offre e, altrimenti, si connette senza cifratura.
    Non controlla il certificato.
tls-require-description =
    Non si connette se il server non offre TLS.
    Non controlla il certificato: protegge da chi ascolta la rete,
    non da chi si spaccia per il server.
tls-verify-description =
    Non si connette senza TLS, e controlla che il certificato sia di quel
    server e che sia firmato da un’autorità di cui il sistema si fida
    o da quella del file della CA indicato.
security-verified = Connessione cifrata (TLS), con il certificato del server verificato.
security-encrypted =
    Connessione cifrata (TLS). Il certificato del server non viene verificato:
    per verificarlo, scegli «{ $verify }» nella connessione.
security-local = Connessione non cifrata, ma non esce da questo computer.
security-plain =
    Connessione NON cifrata: tutto ciò che si invia e si riceve, password compresa,
    può essere letto sulla rete. Il server non offre TLS.
tls-hint-certificate = Non è stato possibile verificare il certificato del server. Se lo firma un’autorità privata, indica il suo file della CA; se non hai bisogno di verificarlo, scegli un altro livello di sicurezza.
tls-hint-required = La sicurezza di questa connessione richiede TLS. Se il server non lo offre, scegli «{ $prefer }».
tls-ca-unreadable = Impossibile leggere il file della CA «{ $path }»: { $error }
tls-ca-empty = non contiene nessun certificato in formato PEM
tls-verifier-failed = Impossibile preparare la verifica del certificato: { $error }
tls-setup-failed = Impossibile preparare TLS: { $error }

## Connecting

keyring-unavailable = nessun portachiavi di sistema disponibile
keyring-locked = il portachiavi è bloccato o lo sblocco è stato annullato ({ $error })
connect-thread-died = Il thread di connessione è terminato in modo inatteso
connect-starting = Avvio...
keyring-save-failed = La password non è stata salvata nel portachiavi: { $error }
connect-reading-keyring = Lettura della password dal portachiavi...
connect-tcp = Apertura della connessione TCP...
connect-authenticating = Autenticazione...
connect-connecting = Connessione al server...
connect-established = Connessione stabilita.
connect-setup-failed = Impossibile preparare la connessione: { $error }
connect-failed = Impossibile connettersi: { $error }{ $hint }
connect-hint-database = Se il server ti lascia entrare solo nel tuo database, indicalo nel campo «Database».
keyring-read-failed = Impossibile leggere la password dal portachiavi: { $error }
keyring-password-gone = La password di questo profilo non è più nel portachiavi di sistema: scrivila e salva di nuovo il profilo.

## Session

session-connecting = Connessione a { $host }...
session-pick-database = Seleziona un database nel pannello a sinistra
session-pick-table = Seleziona una tabella nel pannello a sinistra

## Session tab bar

tab-database = Database
tab-structure = Struttura
tab-structure-of = Struttura di { $table }
tab-data = Dati
tab-rename = Rinomina
tab-close = Chiudi
tab-close-sql-hint = Chiudi la scheda SQL  { $shortcut } (o clic centrale sulla scheda)
tab-new-sql = Nuova scheda SQL
tab-name-hint = Nome della scheda

## Schema tree

tree-title = Schema
tree-showing-favourites = Solo preferiti
tree-filter-favourites = Filtra i preferiti
tree-reload = Ricarica lo schema
tree-search = Cerca…
tree-create-database = Crea database
tree-refresh = Aggiorna
tree-disconnect = Disconnetti
tree-no-matches = (nessuna corrispondenza)
tree-no-tables = (nessuna tabella)
tree-create-table = Crea tabella
tree-export-database = Esporta il database
tree-empty-all-tables = Svuota tutte le tabelle
tree-drop-database = Elimina il database
tree-unfavourite = Rimuovi dai preferiti
tree-favourite = Aggiungi ai preferiti
tree-show-structure = Mostra la struttura
tree-open-in-sql = Apri in SQL
tree-copy-name = Copia il nome
tree-show-definition = Mostra la definizione
tree-export-view = Esporta la vista
tree-drop-view = Elimina la vista
tree-clone-table = Clona la tabella
tree-export-table = Esporta la tabella
tree-empty-table = Svuota la tabella
tree-drop-table = Elimina la tabella

## Server and database panels

server-info-title = Server: { $host }
server-info-database-count = { $count ->
    [one] — { $count } database
   *[other] — { $count } database
}
server-info-no-databases = (nessun database)
server-info-col-database = Database
server-info-col-tables = Tabelle
server-info-col-size = Dimensione
db-info-title = Database: { $db }
db-info-table-count = { $count ->
    [one] — { $count } tabella
   *[other] — { $count } tabelle
}
db-info-no-tables = Questo database non ha tabelle.
db-info-col-table = Tabella
db-info-col-engine = Motore
db-info-col-rows = Righe
db-info-col-size = Dimensione
db-info-open-hint = Doppio clic per aprire la tabella

## Table structure

structure-column-count = { $count ->
    [one] — { $count } colonna
   *[other] — { $count } colonne
}
structure-no-columns = (nessuna colonna)
structure-col-column = Colonna
structure-col-type = Tipo
structure-col-key = Chiave

## Data grid

grid-sort-asc-hint = Clic: ordina ↑
grid-sort-desc-hint = Clic: ordina ↓
grid-sort-off-hint = Clic: rimuovi l’ordinamento
grid-remove-new-row = Rimuovi la nuova riga
grid-running-query = Esecuzione della query…
grid-page-empty = — nessuna riga in questa pagina
grid-page-rows = — righe { $first }–{ $last }
grid-page-size-hint = Si chiedono { $count } righe per pagina (si cambia in Impostazioni).
grid-page-previous = Precedente
grid-page-previous-hint = Pagina precedente
grid-page-next = Successiva
grid-page-next-hint = Pagina successiva
grid-row-count = { $count ->
    [one] — { $count } riga
   *[other] — { $count } righe
}
grid-view-read-only = Vista · sola lettura
grid-view-read-only-hint =
    I dati di una vista sono quelli delle sue tabelle: si modificano lì.
    La sua definizione si apre dal menu della vista nell’albero.
grid-row = Riga
grid-insert-row-hint = Inserisci una riga
grid-no-primary-key = Nessuna chiave primaria
grid-no-primary-key-hint =
    Senza chiave primaria non c’è un modo sicuro per identificare una riga:
    le righe esistenti non si possono modificare né eliminare da qui.
    Si possono comunque aggiungere righe nuove.
grid-sort = Ordine:
grid-sort-remove = Rimuovi «{ $column }»
grid-sort-clear = Cancella tutto
grid-sort-clear-hint = Rimuovi tutto l’ordinamento

## Grid context menu

menu-copy-value = Copia il valore
menu-copy-csv = Copia come CSV
menu-copy-tsv = Copia come TSV
menu-copy-inserts = Copia come SQL INSERT
menu-edit-cell = Modifica la cella
menu-set-null = Imposta NULL
menu-set-empty = Imposta vuoto
menu-now = Data/ora attuale
menu-default-value = Valore predefinito
menu-undo-change = Annulla la modifica
menu-filter-clipboard = Dagli appunti
menu-filter-text = Per testo...
menu-insert-row = Inserisci riga
menu-clone-row = Clona la riga
menu-undelete = Annulla l’eliminazione
menu-delete-rows = { $count ->
    [one] Elimina la riga
   *[other] Elimina { $count } righe
}

## Filter

filter-history-hint = Filtri recenti
filter-no-history = Nessuna cronologia
filter-hint = id = 1 AND nome LIKE 'foo%'
filter-field-hint = SQL diretto — aggiunto alla query come clausola WHERE
filter-apply = Applica
filter-clear = Cancella il filtro
filter-by-title = Filtra per { $column }
filter-operator = Operatore:
filter-value = Valore:
filter-value-hint = valore da filtrare
filter-op-equals = uguale a
filter-op-not-equals = diverso da
filter-op-contains = contiene
filter-op-starts-with = inizia con
filter-op-ends-with = finisce con
filter-op-greater = maggiore di
filter-op-less = minore di
filter-op-greater-or-equal = maggiore o uguale
filter-op-less-or-equal = minore o uguale

## Pending changes and saving

apply-no-primary-key = Nessuna chiave primaria — impossibile salvare
apply-edits = { $count ->
    [one] { $count } modifica
   *[other] { $count } modifiche
}
apply-deletes = { $count ->
    [one] { $count } eliminazione
   *[other] { $count } eliminazioni
}
apply-inserts = { $count ->
    [one] { $count } inserimento
   *[other] { $count } inserimenti
}
apply-discard = Scarta
apply-save = Salva
save-no-open-table = Nessuna tabella aperta.
save-no-data = Nessun dato caricato.
save-structure-loading = La struttura della tabella è ancora in lettura: riprova.
save-no-primary-key = La tabella non ha una chiave primaria: le righe non si possono modificare né eliminare.
save-no-column = La colonna { $index } non esiste.
save-cannot-title = Impossibile salvare le modifiche
save-failed-title = Errore durante il salvataggio
log-save-changes = Salva modifiche: { $deletes } eliminazione/i, { $edits } modifica/che, { $inserts } inserimento/i (transazione)
changes-saved = { $count ->
    [one] { $count } modifica salvata in «{ $table }»
   *[other] { $count } modifiche salvate in «{ $table }»
}
save-row-key-missing = Impossibile trovare la chiave primaria della riga { $row }.
save-row-not-found = Non è stato salvato nulla: un’istruzione doveva modificare una riga e ne ha trovate { $found }. La riga potrebbe essere cambiata o sparita sul server; ricarica la tabella (F5) e ripeti le modifiche.

## SQL editor

sql-tab-title = SQL { $number }
sql-database = DB:
sql-no-database = (nessuno)
sql-free-query = SQL libero:
sql-run-selection = Esegui la selezione
sql-run = Esegui
sql-clear = Pulisci
sql-cancelling = Annullamento…
sql-running = Esecuzione…
sql-cancel-hint = Interrompe la query sul server
sql-editor-hint = SELECT * FROM tabella LIMIT 100
sql-empty = Scrivi una query e premi { $shortcut } o il pulsante Esegui
sql-cancelled = Query annullata
sql-done-no-result = Eseguita, nessun risultato da mostrare
sql-result-summary = { $rows ->
    [one] { $rows } riga
   *[other] { $rows } righe
}, { $columns ->
    [one] { $columns } colonna
   *[other] { $columns } colonne
}
sql-result-truncated = solo le prime { $count } (ce ne sono altre; il limite si cambia in Impostazioni)

## Query log

log-title = Registro delle query
log-entry-count = { $count ->
    [one] { $count } voce
   *[other] { $count } voci
}
log-cancelled = ANNULLATA
log-rows = { $count ->
    [one] { $count } riga
   *[other] { $count } righe
}
log-copy-query = Copia la query
log-copy-error = Copia l’errore
log-open-in-sql-tab = Apri in una scheda SQL
log-error = ERRORE
log-cancel-query = Annulla la query
log-export = Esportazione
log-export-finished = { $count ->
    [one] Esportazione terminata: { $count } tabella
   *[other] Esportazione terminata: { $count } tabelle
}

## Create, drop and empty dialogs

discard-sql-close-title = Chiudi «{ $title }»
discard-sql-close = Chiudi la scheda
discard-sql-clear-title = Pulisci «{ $title }»
discard-sql-warning = La query di questa scheda andrà persa:
create = Crea
create-name = Nome:
create-database-name-hint = mio_database
create-table-name = Nome della tabella:
create-table-name-hint = mia_tabella
create-columns = Colonne:
create-col-name = Nome
create-col-length = Lunghezza
create-add-column = Aggiungi colonna
drop-confirm-database = Vuoi davvero eliminare il database:
drop-confirm-view = Vuoi davvero eliminare la vista:
drop-confirm-table = Vuoi davvero eliminare la tabella:
drop-cannot-undo = Questa azione non si può annullare.
drop-type-name = Scrivi il suo nome per confermare:
truncate-title = Svuota i dati
truncate-confirm-all = Vuoi davvero svuotare tutte le tabelle di:
truncate-confirm-table = Vuoi davvero svuotare la tabella:
truncate-warning = Tutti i dati verranno eliminati. Non si può annullare.
truncate-button = Svuota
unsaved-title = Modifiche non salvate
unsaved-changes = { $count ->
    [one] Hai { $count } modifica non salvata in «{ $table }».
   *[other] Hai { $count } modifiche non salvate in «{ $table }».
}
unsaved-switch-question = Cosa vuoi fare prima di aprire «{ $table }»?
unsaved-save-and-switch = Salva e cambia
unsaved-discard-and-switch = Scarta e cambia
unsaved-lost-on-page = Cambiando pagina andrebbero perse.
unsaved-lost-on-reload = Ricaricando i dati andrebbero perse.
unsaved-save-and-reload = Salva e ricarica
unsaved-discard-and-reload = Scarta e ricarica
unsaved-lost-on-close = Chiudendo la scheda andranno perse.
unsaved-discard-and-close = Scarta e chiudi
clone-title = Clona la tabella — { $table }
clone-new-name = Nuovo nome:
clone-name-hint = { $table }_copia
clone-copy-data = Copia i dati:
clone-button = Clona

## Favourites

favourite-deleted = Preferito «{ $name }» eliminato
favourite-saved = Preferito «{ $name }» salvato
favourite-name-missing = Dai un nome al preferito per salvarlo.
favourite-name-taken = Esiste già un preferito chiamato «{ $name }»: cambia il nome per salvarlo come un altro.
favourite-delete-title = Elimina il preferito
favourite-delete-question = Vuoi davvero eliminare il preferito:
favourite-delete-keyring = Verrà rimossa anche la sua password dal portachiavi.

## Notifications

database-list-failed = Impossibile leggere l’elenco dei database.
clipboard-read-failed = Impossibile leggere gli appunti: { $error }
name-copied = «{ $name }» copiato negli appunti
rows-copied = { $count ->
    [one] { $count } riga copiata ({ $format })
   *[other] { $count } righe copiate ({ $format })
}
database-created = Database «{ $db }» creato
database-dropped = Database «{ $db }» eliminato
database-drop-failed = Impossibile eliminare «{ $db }»
table-created = Tabella «{ $table }» creata
table-dropped = «{ $table }» eliminata
table-drop-failed = Impossibile eliminare «{ $table }»
tables-list-failed = Impossibile elencare le tabelle di «{ $db }»
tables-empty-failed = Impossibile svuotare tutte le tabelle di «{ $db }»
table-emptied = Tabella «{ $table }» svuotata
tables-emptied = { $count ->
    [one] { $count } tabella di «{ $db }» svuotata
   *[other] { $count } tabelle di «{ $db }» svuotate
}

## Export dialog

export-title-table = Esporta «{ $table }»
export-thread-died = Il thread di esportazione è terminato in modo inatteso.
export-title = Esporta i dati — { $db }
export-tables-heading = Tabelle ({ $selected }/{ $total })
export-select-all = Tutte
export-select-none = Nessuna
export-destination = Destinazione
export-destination-label = Destinazione:
export-to-file = File SQL (.sql)
export-to-folder = Cartella (un .sql per tabella)
export-to-database = Un altro database (stesso server)
export-to-server = Un altro server (preferiti)
export-choose = Seleziona…
export-no-profiles = Nessun profilo
export-no-profiles-hint = Salva prima un profilo con il server di destinazione.
export-target-database = DB di destinazione:
export-options = Opzioni
export-include-create = Includi CREATE TABLE
export-include-data = Includi i dati (INSERT INTO)
export-button = Esporta
export-select-a-table = Seleziona almeno una tabella
export-progress = { $done }/{ $total } tabelle
export-tables-done = Tabelle completate: { $done } / { $total }
export-rows-done = Righe esportate:   { $rows }
export-rows-unit = righe
export-cancel = Annulla l’esportazione
export-completed = Completato
export-completed-errors = { $count ->
    [one] Completato con { $count } errore
   *[other] Completato con { $count } errori
}
export-done = Esportazione completata
export-done-errors = Esportazione completata con errori
export-summary-tables = Tabelle: { $count }
export-summary-errors = Con errori: { $count } (dettagli nel registro)
export-summary-rows = Righe:  { $rows }
export-summary-time = Tempo: { $seconds } s
export-cancelled = Annullato
export-failed = L’esportazione non è riuscita:
export-same-target = La destinazione è lo stesso database dello stesso server dell’origine: la copia cancellerebbe o duplicherebbe i suoi dati. Scegli un altro database di destinazione.
export-source-connection = Connessione all’origine: { $error }
export-target-connection = Connessione alla destinazione: { $error }
export-same-server-check-failed = Impossibile verificare se la destinazione è lo stesso server: { $error }
export-create-target-failed = Impossibile creare il database di destinazione «{ $db }»: { $error }
export-file-create-failed = Impossibile creare il file «{ $path }»: { $error }
export-file-write-failed = Errore durante la scrittura del file: { $error }
export-folder-create-failed = Impossibile creare la cartella «{ $folder }»: { $error }
export-cancelled-error = Esportazione annullata.
export-drop-existing-failed =
    Impossibile eliminare dalla destinazione le tabelle che esistono già: { $error }

    Se altre tabelle o viste della destinazione dipendono da esse, esporta anche quelle o eliminale prima.
export-no-ddl = Nessun DDL ottenuto per la tabella «{ $table }»
export-table-gone = La tabella «{ $table }» non esiste più.
export-foreign-table = Le tabelle esterne non vengono esportate.
export-dump-header = Esportazione RustQL — Database: { $db }
export-dump-generated = Generato: { $date }
export-dump-table = Tabella: { $table }
export-dump-deferred = Viste e chiavi esterne
export-partitioned-table = Le tabelle partizionate non vengono esportate: le loro partizioni non verrebbero copiate. Usa pg_dump.
export-postgres-scope = Si copiano tabelle e viste con le loro sequenze, chiavi e indici. Non trigger, funzioni, permessi né partizioni: per una copia completa, usa pg_dump.

## Database errors

view-definition-failed = Impossibile leggere la definizione di «{ $view }»: { $error }
cancel-query-failed = Impossibile annullare la query: { $error }
db-no-connection = Nessuna connessione con il server: { $error }
db-no-connection-to = Nessuna connessione con «{ $db }»: { $error }
db-query-error = Errore nella query: { $error }
db-write-error = Errore di scrittura: { $error }
db-begin-error = Impossibile avviare la transazione: { $error }
db-transaction-error = Errore nella transazione (ROLLBACK): { $error }
db-commit-error = Errore nel COMMIT: { $error }
editor-session-failed = Impossibile aprire la sessione dell’editor: { $error }
view-definition-missing = Impossibile trovare la definizione della vista «{ $view }».
view-materialized-comment =
    -- Una vista materializzata non si può sostituire: per cambiarla
    -- bisogna eliminarla e ricrearla.
charset-invalid = Charset non valido: «{ $charset }». Sono ammessi solo caratteri alfanumerici e «_».
