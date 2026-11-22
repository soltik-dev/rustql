# RustQL — English, the reference language: every text exists here first.
# Ids are grouped by the part of the interface they belong to.

## Shared

common-cancel = Cancel
common-error = Error: { $error }
common-loading = Loading…
error-title = Error
toast-close-hint = Click to close
common-close = Close
common-delete = Delete

## Keys (as printed on the keyboard)

key-delete = Del
key-page-up = PgUp
key-page-down = PgDn

## Status bar

status-schema = Schema
status-log = Log
status-settings = Settings
status-disconnected = Not connected
status-connecting = Connecting to { $host }…

## Settings panel

settings-title = Settings
settings-language = Language
settings-language-system = System
settings-theme = Theme
settings-theme-system = System
settings-theme-light = Light
settings-theme-dark = Dark
settings-font = Font
settings-font-embedded = Inter (built in)
settings-font-size = Font size
settings-font-sample = Sample text
settings-copy-format = Copy format (several rows)
settings-page-rows = Rows per page (tables)
settings-editor-max-rows = Max. rows in the SQL editor

## Connection tabs

conn-tabs-new = New connection
conn-tabs-close = Close connection

## Connection form

conn-favourites = Favourites
conn-no-favourites = No saved connections.
conn-no-favourites-hint =
    Fill in the form
    and press Save.
conn-subtitle = Connect to a database
conn-engine = Engine:
conn-host = Host:
conn-port = Port:
conn-user = User:
conn-password = Password:
conn-password-in-keyring = Saved in the keyring
conn-database = Database:
conn-database-all = All
conn-database-pick = Choose among the server's databases
conn-database-reading = Reading the databases…
conn-security = Security:
conn-ca-file = CA file:
conn-ca-pick = Choose the file
conn-ca-filter-certificates = Certificates
conn-ca-filter-all = All
conn-ca-system = The system's
conn-ca-hint =
    Certificate (PEM) of the authority that signs the server's.
    Empty, the authorities installed on the system are trusted.
conn-save-password = Save password
conn-no-keyring = No system keyring: it will not be saved
conn-save-password-hint = Stored encrypted in the system keyring
conn-profile-name = Name:
conn-profile-name-hint = My server...
conn-color = Colour:
conn-color-none = No colour
conn-color-custom = Custom…
conn-save = Save
conn-save-as-new = Save as new
conn-save-as-new-hint = Creates another favourite with these details, leaving the selected one untouched
conn-delete = Delete
conn-connect = Connect
conn-port-invalid = Invalid port: “{ $port }”. It must be a number between 1 and 65535.
conn-host-missing = Enter the server's host.

## Connection colours

color-blue = Blue
color-teal = Teal
color-green = Green
color-yellow = Yellow
color-orange = Orange
color-red = Red
color-pink = Pink
color-purple = Purple
color-custom = Custom

## Connection security

tls-prefer = Encrypt if possible
tls-require = Require encryption
tls-verify = Verify the server
tls-prefer-description =
    Uses TLS if the server offers it and, if not, connects unencrypted.
    It does not check the certificate.
tls-require-description =
    Does not connect if the server does not offer TLS.
    It does not check the certificate: it protects against eavesdroppers,
    not against someone impersonating the server.
tls-verify-description =
    Does not connect without TLS, and checks that the certificate belongs to
    that server and is signed by an authority the system trusts
    or by the one in the CA file you give.
security-verified = Encrypted connection (TLS), with the server certificate verified.
security-encrypted =
    Encrypted connection (TLS). The server certificate is not verified:
    to verify it, choose “{ $verify }” in the connection.
security-local = Unencrypted connection, but it never leaves this machine.
security-plain =
    UNENCRYPTED connection: whatever is sent and received, password included,
    can be read on the network. The server does not offer TLS.
tls-hint-certificate = The server certificate could not be verified. If a private authority signs it, give its CA file; if you do not need to verify it, choose another security level.
tls-hint-required = This connection's security requires TLS. If the server does not offer it, choose “{ $prefer }”.
tls-ca-unreadable = Could not read the CA file “{ $path }”: { $error }
tls-ca-empty = it contains no certificate in PEM format
tls-verifier-failed = Could not set up the certificate verification: { $error }
tls-setup-failed = Could not set up TLS: { $error }

## Connecting

keyring-unavailable = no system keyring is available
keyring-locked = the keyring is locked or unlocking was cancelled ({ $error })
connect-thread-died = The connection thread ended unexpectedly
connect-starting = Starting...
keyring-save-failed = The password was not saved in the keyring: { $error }
connect-reading-keyring = Reading the password from the keyring...
connect-tcp = Opening the TCP connection...
connect-authenticating = Authenticating...
connect-connecting = Connecting to the server...
connect-established = Connection established.
connect-setup-failed = Could not set up the connection: { $error }
connect-failed = Could not connect: { $error }{ $hint }
connect-hint-database = If the server only lets you into your own database, enter it in the “Database” field.
keyring-read-failed = Could not read the password from the keyring: { $error }
keyring-password-gone = This profile's password is no longer in the system keyring: type it and save the profile again.

## Session

session-connecting = Connecting to { $host }...
session-pick-database = Select a database in the left panel
session-pick-table = Select a table in the left panel

## Session tab bar

tab-database = Database
tab-structure = Structure
tab-structure-of = Structure of { $table }
tab-data = Data
tab-rename = Rename
tab-close = Close
tab-close-sql-hint = Close SQL tab  { $shortcut } (or middle-click the tab)
tab-new-sql = New SQL tab
tab-name-hint = Tab name

## Schema tree

tree-title = Schema
tree-showing-favourites = Showing favourites only
tree-filter-favourites = Filter favourites
tree-reload = Reload schema
tree-search = Search…
tree-create-database = Create database
tree-refresh = Refresh
tree-disconnect = Disconnect
tree-no-matches = (no matches)
tree-no-tables = (no tables)
tree-create-table = Create table
tree-export-database = Export database
tree-empty-all-tables = Empty all tables
tree-drop-database = Drop database
tree-unfavourite = Remove from favourites
tree-favourite = Add to favourites
tree-show-structure = Show structure
tree-open-in-sql = Open in SQL
tree-copy-name = Copy name
tree-show-definition = Show definition
tree-export-view = Export view
tree-drop-view = Drop view
tree-clone-table = Clone table
tree-export-table = Export table
tree-empty-table = Empty table
tree-drop-table = Drop table

## Server and database panels

server-info-title = Server: { $host }
server-info-database-count = { $count ->
    [one] — 1 database
   *[other] — { $count } databases
}
server-info-no-databases = (no databases)
server-info-col-database = Database
server-info-col-tables = Tables
server-info-col-size = Size
db-info-title = Database: { $db }
db-info-table-count = { $count ->
    [one] — 1 table
   *[other] — { $count } tables
}
db-info-no-tables = This database has no tables.
db-info-col-table = Table
db-info-col-engine = Engine
db-info-col-rows = Rows
db-info-col-size = Size
db-info-open-hint = Double-click to open the table

## Table structure

structure-column-count = { $count ->
    [one] — 1 column
   *[other] — { $count } columns
}
structure-no-columns = (no columns)
structure-col-column = Column
structure-col-type = Type
structure-col-key = Key

## Data grid

grid-sort-asc-hint = Click: sort ↑
grid-sort-desc-hint = Click: sort ↓
grid-sort-off-hint = Click: remove sorting
grid-remove-new-row = Remove the new row
grid-running-query = Running query…
grid-page-empty = — no rows on this page
grid-page-rows = — rows { $first }–{ $last }
grid-page-size-hint = { $count } rows are requested per page (change it in Settings).
grid-page-previous = Previous
grid-page-previous-hint = Previous page
grid-page-next = Next
grid-page-next-hint = Next page
grid-row-count = { $count ->
    [one] — 1 row
   *[other] — { $count } rows
}
grid-view-read-only = View · read only
grid-view-read-only-hint =
    A view's data is its tables': edit it there.
    Its definition opens from the view's menu in the tree.
grid-row = Row
grid-insert-row-hint = Insert a row
grid-no-primary-key = No primary key
grid-no-primary-key-hint =
    Without a primary key there is no safe way to identify a row:
    existing rows cannot be edited or deleted from here.
    New rows can still be added.
grid-sort = Sort:
grid-sort-remove = Remove “{ $column }”
grid-sort-clear = Clear all
grid-sort-clear-hint = Remove all sorting

## Grid context menu

menu-copy-value = Copy value
menu-copy-csv = Copy as CSV
menu-copy-tsv = Copy as TSV
menu-copy-inserts = Copy as SQL INSERT
menu-edit-cell = Edit cell
menu-set-null = Set NULL
menu-set-empty = Set empty
menu-now = Current date/time
menu-default-value = Default value
menu-undo-change = Undo the change
menu-filter-clipboard = By clipboard
menu-filter-text = By text...
menu-insert-row = Insert row
menu-clone-row = Clone row
menu-undelete = Undo deletion
menu-delete-rows = { $count ->
    [one] Delete row
   *[other] Delete { $count } rows
}

## Filter

filter-history-hint = Recent filters
filter-no-history = No history
filter-hint = id = 1 AND name LIKE 'foo%'
filter-field-hint = Plain SQL — added to the query as its WHERE clause
filter-apply = Apply
filter-clear = Clear filter
filter-by-title = Filter by { $column }
filter-operator = Operator:
filter-value = Value:
filter-value-hint = value to filter by
filter-op-equals = equal to
filter-op-not-equals = not equal to
filter-op-contains = contains
filter-op-starts-with = starts with
filter-op-ends-with = ends with
filter-op-greater = greater than
filter-op-less = less than
filter-op-greater-or-equal = greater or equal
filter-op-less-or-equal = less or equal

## Pending changes and saving

apply-no-primary-key = No primary key — changes cannot be saved
apply-edits = { $count ->
    [one] 1 edit
   *[other] { $count } edits
}
apply-deletes = { $count ->
    [one] 1 deletion
   *[other] { $count } deletions
}
apply-inserts = { $count ->
    [one] 1 insertion
   *[other] { $count } insertions
}
apply-discard = Discard
apply-save = Save
save-no-open-table = No table is open.
save-no-data = No data is loaded.
save-structure-loading = The table structure is still being read: try again.
save-no-primary-key = The table has no primary key: rows cannot be edited or deleted.
save-no-column = Column { $index } does not exist.
save-cannot-title = The changes cannot be saved
save-failed-title = Error while saving
log-save-changes = Save changes: { $deletes } deletion(s), { $edits } edit(s), { $inserts } insertion(s) (transaction)
changes-saved = { $count ->
    [one] 1 change saved in “{ $table }”
   *[other] { $count } changes saved in “{ $table }”
}
save-row-key-missing = The primary key of row { $row } cannot be found.
save-row-not-found = Nothing was saved: a statement had to change one row and found { $found }. The row may have changed or disappeared on the server; reload the table (F5) and make the changes again.

## SQL editor

sql-tab-title = SQL { $number }
sql-database = Database:
sql-no-database = (none)
sql-free-query = Free SQL:
sql-run-selection = Run selection
sql-run = Run
sql-clear = Clear
sql-cancelling = Cancelling…
sql-running = Running…
sql-cancel-hint = Stops the query on the server
sql-editor-hint = SELECT * FROM table LIMIT 100
sql-empty = Write a query and press { $shortcut } or the Run button
sql-cancelled = Query cancelled
sql-done-no-result = Done, no result to show
sql-result-summary = { $rows ->
    [one] 1 row
   *[other] { $rows } rows
}, { $columns ->
    [one] 1 column
   *[other] { $columns } columns
}
sql-result-truncated = only the first { $count } (there are more; the limit is in Settings)

## Query log

log-title = Query log
log-entry-count = { $count ->
    [one] 1 entry
   *[other] { $count } entries
}
log-cancelled = CANCELLED
log-rows = { $count ->
    [one] 1 row
   *[other] { $count } rows
}
log-copy-query = Copy the query
log-copy-error = Copy the error
log-open-in-sql-tab = Open in a SQL tab
log-error = ERROR
log-cancel-query = Cancel the query
log-export = Export
log-export-finished = { $count ->
    [one] Export finished: 1 table
   *[other] Export finished: { $count } tables
}

## Create, drop and empty dialogs

discard-sql-close-title = Close “{ $title }”
discard-sql-close = Close tab
discard-sql-clear-title = Clear “{ $title }”
discard-sql-warning = The query in this tab will be lost:
create = Create
create-name = Name:
create-database-name-hint = my_database
create-table-name = Table name:
create-table-name-hint = my_table
create-columns = Columns:
create-col-name = Name
create-col-length = Length
create-add-column = Add column
drop-confirm-database = Are you sure you want to drop the database:
drop-confirm-view = Are you sure you want to drop the view:
drop-confirm-table = Are you sure you want to drop the table:
drop-cannot-undo = This cannot be undone.
drop-type-name = Type its name to confirm:
truncate-title = Empty data
truncate-confirm-all = Are you sure you want to empty all the tables of:
truncate-confirm-table = Are you sure you want to empty the table:
truncate-warning = All the data will be deleted. This cannot be undone.
truncate-button = Empty
unsaved-title = Unsaved changes
unsaved-changes = { $count ->
    [one] You have 1 unsaved change in “{ $table }”.
   *[other] You have { $count } unsaved changes in “{ $table }”.
}
unsaved-switch-question = What do you want to do before opening “{ $table }”?
unsaved-save-and-switch = Save and switch
unsaved-discard-and-switch = Discard and switch
unsaved-lost-on-page = Changing page would lose them.
unsaved-lost-on-reload = Reloading the data would lose them.
unsaved-save-and-reload = Save and reload
unsaved-discard-and-reload = Discard and reload
unsaved-lost-on-close = Closing the tab will lose them.
unsaved-discard-and-close = Discard and close
clone-title = Clone table — { $table }
clone-new-name = New name:
clone-name-hint = { $table }_copy
clone-copy-data = Copy data:
clone-button = Clone

## Favourites

favourite-deleted = Favourite “{ $name }” deleted
favourite-saved = Favourite “{ $name }” saved
favourite-name-missing = Give the favourite a name to save it.
favourite-name-taken = There is already a favourite called “{ $name }”: change the name to save it as another one.
favourite-delete-title = Delete favourite
favourite-delete-question = Are you sure you want to delete the favourite:
favourite-delete-keyring = Its password will also be removed from the keyring.

## Notifications

database-list-failed = Could not read the list of databases.
clipboard-read-failed = Could not read the clipboard: { $error }
name-copied = “{ $name }” copied to the clipboard
rows-copied = { $count ->
    [one] 1 row copied ({ $format })
   *[other] { $count } rows copied ({ $format })
}
database-created = Database “{ $db }” created
database-dropped = Database “{ $db }” dropped
database-drop-failed = Could not drop “{ $db }”
table-created = Table “{ $table }” created
table-dropped = “{ $table }” dropped
table-drop-failed = Could not drop “{ $table }”
tables-list-failed = Could not list the tables of “{ $db }”
tables-empty-failed = Could not empty all the tables of “{ $db }”
table-emptied = Table “{ $table }” emptied
tables-emptied = { $count ->
    [one] 1 table of “{ $db }” emptied
   *[other] { $count } tables of “{ $db }” emptied
}

## Export dialog

export-title-table = Export “{ $table }”
export-thread-died = The export thread ended unexpectedly.
export-title = Export data — { $db }
export-tables-heading = Tables ({ $selected }/{ $total })
export-select-all = All
export-select-none = None
export-destination = Destination
export-destination-label = Destination:
export-to-file = SQL file (.sql)
export-to-folder = Folder (one .sql per table)
export-to-database = Another database (same server)
export-to-server = Another server (favourites)
export-choose = Choose…
export-no-profiles = No profiles
export-no-profiles-hint = Save a profile with the destination server first.
export-target-database = Target database:
export-options = Options
export-include-create = Include CREATE TABLE
export-include-data = Include data (INSERT INTO)
export-button = Export
export-select-a-table = Select at least one table
export-progress = { $done }/{ $total } tables
export-tables-done = Tables done: { $done } / { $total }
export-rows-done = Rows exported:   { $rows }
export-rows-unit = rows
export-cancel = Cancel export
export-completed = Completed
export-completed-errors = { $count ->
    [one] Completed with 1 error
   *[other] Completed with { $count } errors
}
export-done = Export completed
export-done-errors = Export completed with errors
export-summary-tables = Tables: { $count }
export-summary-errors = With errors: { $count } (details in the log)
export-summary-rows = Rows:  { $rows }
export-summary-time = Time: { $seconds } s
export-cancelled = Cancelled
export-failed = The export failed:
export-same-target = The destination is the same database on the same server as the source: the copy would delete or duplicate its data. Choose another destination database.
export-source-connection = Connection to the source: { $error }
export-target-connection = Connection to the destination: { $error }
export-same-server-check-failed = Could not check whether the destination is the same server: { $error }
export-create-target-failed = Could not create the destination database “{ $db }”: { $error }
export-file-create-failed = Could not create the file “{ $path }”: { $error }
export-file-write-failed = Error writing the file: { $error }
export-folder-create-failed = Could not create the folder “{ $folder }”: { $error }
export-cancelled-error = Export cancelled.
export-drop-existing-failed =
    Could not drop the tables that already exist at the destination: { $error }

    If other tables or views at the destination depend on them, export those too or drop them first.
export-no-ddl = No DDL was obtained for the table “{ $table }”
export-table-gone = The table “{ $table }” no longer exists.
export-foreign-table = Foreign tables are not exported.
export-dump-header = RustQL Export — Database: { $db }
export-dump-generated = Generated: { $date }
export-dump-table = Table: { $table }
export-dump-deferred = Views and foreign keys
export-partitioned-table = Partitioned tables are not exported: their partitions would not be copied. Use pg_dump.
export-postgres-scope = Tables and views are copied with their sequences, keys and indexes. Not triggers, functions, privileges or partitions: for a full copy, use pg_dump.

## Database errors

view-definition-failed = Could not read the definition of “{ $view }”: { $error }
cancel-query-failed = Could not cancel the query: { $error }
db-no-connection = No connection to the server: { $error }
db-no-connection-to = No connection to “{ $db }”: { $error }
db-query-error = Query error: { $error }
db-write-error = Write error: { $error }
db-begin-error = Could not start the transaction: { $error }
db-transaction-error = Error in the transaction (ROLLBACK): { $error }
db-commit-error = Error in COMMIT: { $error }
editor-session-failed = Could not open the editor session: { $error }
view-definition-missing = The definition of the view “{ $view }” cannot be found.
view-materialized-comment =
    -- A materialized view cannot be replaced: to change it, drop it
    -- and create it again.
charset-invalid = Invalid charset: “{ $charset }”. Only alphanumeric characters and “_” are allowed.
