# RustQL — español.

## Shared

common-cancel = Cancelar
common-error = Error: { $error }
common-loading = Cargando…
error-title = Error
toast-close-hint = Clic para cerrar
common-close = Cerrar
common-delete = Eliminar

## Keys (as printed on the keyboard)

key-delete = Supr
key-page-up = Re Pág
key-page-down = Av Pág

## Status bar

status-schema = Esquema
status-log = Log
status-settings = Ajustes
status-disconnected = Sin conexión
status-connecting = Conectando a { $host }…

## Settings panel

settings-title = Ajustes
settings-language = Idioma
settings-language-system = Sistema
settings-theme = Tema
settings-theme-system = Sistema
settings-theme-light = Claro
settings-theme-dark = Oscuro
settings-font = Fuente
settings-font-embedded = Inter (integrada)
settings-font-size = Tamaño de fuente
settings-font-sample = Texto de muestra
settings-copy-format = Formato de copia (varias filas)
settings-page-rows = Filas por página (tablas)
settings-editor-max-rows = Máx. filas en el editor SQL

## Connection tabs

conn-tabs-new = Nueva conexión
conn-tabs-close = Cerrar conexión

## Connection form

conn-favourites = Favoritos
conn-no-favourites = Sin conexiones guardadas.
conn-no-favourites-hint =
    Rellena el formulario
    y pulsa Guardar.
conn-subtitle = Conectar a base de datos
conn-engine = Motor:
conn-host = Host:
conn-port = Puerto:
conn-user = Usuario:
conn-password = Contraseña:
conn-password-in-keyring = Guardada en el llavero
conn-database = Base de datos:
conn-database-all = Todas
conn-database-pick = Elegir entre las bases de datos del servidor
conn-database-reading = Leyendo las bases de datos…
conn-security = Seguridad:
conn-ca-file = Fichero de CA:
conn-ca-pick = Elegir el fichero
conn-ca-filter-certificates = Certificados
conn-ca-filter-all = Todos
conn-ca-system = Las del sistema
conn-ca-hint =
    Certificado (PEM) de la autoridad que firma el del servidor.
    Vacío, se confía en las autoridades instaladas en el sistema.
conn-save-password = Guardar contraseña
conn-no-keyring = Sin llavero del sistema: no se guardará
conn-save-password-hint = Se guarda cifrada en el llavero del sistema
conn-profile-name = Nombre:
conn-profile-name-hint = Mi servidor...
conn-color = Color:
conn-color-none = Sin color
conn-color-custom = Personalizado…
conn-save = Guardar
conn-save-as-new = Guardar como nuevo
conn-save-as-new-hint = Crea otro favorito con estos datos, sin tocar el seleccionado
conn-delete = Eliminar
conn-connect = Conectar
conn-port-invalid = Puerto no válido: «{ $port }». Debe ser un número entre 1 y 65535.
conn-host-missing = Indica el host del servidor.

## Connection colours

color-blue = Azul
color-teal = Turquesa
color-green = Verde
color-yellow = Amarillo
color-orange = Naranja
color-red = Rojo
color-pink = Rosa
color-purple = Morado
color-custom = Personalizado

## Connection security

tls-prefer = Cifrar si se puede
tls-require = Exigir cifrado
tls-verify = Verificar el servidor
tls-prefer-description =
    Usa TLS si el servidor lo ofrece y, si no, conecta sin cifrar.
    No comprueba el certificado.
tls-require-description =
    No conecta si el servidor no ofrece TLS.
    No comprueba el certificado: protege de quien escucha la red,
    no de quien suplanta al servidor.
tls-verify-description =
    No conecta sin TLS, y comprueba que el certificado es de ese
    servidor y lo firma una autoridad en la que confía el sistema
    o la del fichero de CA que indiques.
security-verified = Conexión cifrada (TLS), con el certificado del servidor verificado.
security-encrypted =
    Conexión cifrada (TLS). El certificado del servidor no se verifica:
    para hacerlo, elige «{ $verify }» en la conexión.
security-local = Conexión sin cifrar, pero no sale de este equipo.
security-plain =
    Conexión SIN cifrar: lo que se envía y se recibe, contraseña incluida,
    puede leerse en la red. El servidor no ofrece TLS.
tls-hint-certificate = No se ha podido verificar el certificado del servidor. Si lo firma una autoridad propia, indica su fichero de CA; si no necesitas verificarlo, elige otro nivel de seguridad.
tls-hint-required = La seguridad de esta conexión exige TLS. Si el servidor no lo ofrece, elige «{ $prefer }».
tls-ca-unreadable = No se pudo leer el fichero de CA «{ $path }»: { $error }
tls-ca-empty = no contiene ningún certificado en formato PEM
tls-verifier-failed = No se pudo preparar la verificación del certificado: { $error }
tls-setup-failed = No se pudo preparar TLS: { $error }

## Connecting

keyring-unavailable = no hay llavero del sistema disponible
keyring-locked = el llavero está bloqueado o se canceló el desbloqueo ({ $error })
connect-thread-died = El hilo de conexión terminó inesperadamente
connect-starting = Iniciando...
keyring-save-failed = No se guardó la contraseña en el llavero: { $error }
connect-reading-keyring = Leyendo la contraseña del llavero...
connect-tcp = Estableciendo conexión TCP...
connect-authenticating = Autenticando...
connect-connecting = Conectando con el servidor...
connect-established = Conexión establecida.
connect-setup-failed = No se pudo preparar la conexión: { $error }
connect-failed = No se pudo conectar: { $error }{ $hint }
connect-hint-database = Si el servidor solo te deja entrar en tu base de datos, indícala en el campo «Base de datos».
keyring-read-failed = No se pudo leer la contraseña del llavero: { $error }
keyring-password-gone = La contraseña de este perfil ya no está en el llavero del sistema: escríbela y vuelve a guardar el perfil.

## Session

session-connecting = Conectando a { $host }...
session-pick-database = Selecciona una base de datos en el panel izquierdo
session-pick-table = Selecciona una tabla en el panel izquierdo

## Session tab bar

tab-database = BBDD
tab-structure = Estructura
tab-structure-of = Estructura de { $table }
tab-data = Datos
tab-rename = Renombrar
tab-close = Cerrar
tab-close-sql-hint = Cerrar pestaña SQL  { $shortcut } (o clic central en la pestaña)
tab-new-sql = Nueva pestaña SQL
tab-name-hint = Nombre de la pestaña

## Schema tree

tree-title = Esquema
tree-showing-favourites = Mostrando solo favoritos
tree-filter-favourites = Filtrar favoritos
tree-reload = Recargar esquema
tree-search = Buscar…
tree-create-database = Crear base de datos
tree-refresh = Refrescar
tree-disconnect = Desconectar
tree-no-matches = (sin coincidencias)
tree-no-tables = (sin tablas)
tree-create-table = Crear tabla
tree-export-database = Exportar base de datos
tree-empty-all-tables = Vaciar todas las tablas
tree-drop-database = Eliminar base de datos
tree-unfavourite = Quitar de favoritos
tree-favourite = Marcar favorito
tree-show-structure = Ver estructura
tree-open-in-sql = Abrir en SQL
tree-copy-name = Copiar nombre
tree-show-definition = Ver definición
tree-export-view = Exportar vista
tree-drop-view = Eliminar vista
tree-clone-table = Clonar tabla
tree-export-table = Exportar tabla
tree-empty-table = Vaciar tabla
tree-drop-table = Eliminar tabla

## Server and database panels

server-info-title = Servidor: { $host }
server-info-database-count = { $count ->
    [one] — 1 base de datos
   *[other] — { $count } bases de datos
}
server-info-no-databases = (sin bases de datos)
server-info-col-database = Base de datos
server-info-col-tables = Tablas
server-info-col-size = Tamaño
db-info-title = Base de datos: { $db }
db-info-table-count = { $count ->
    [one] — 1 tabla
   *[other] — { $count } tablas
}
db-info-no-tables = Esta base de datos no tiene tablas.
db-info-col-table = Tabla
db-info-col-engine = Motor
db-info-col-rows = Filas
db-info-col-size = Tamaño
db-info-open-hint = Doble clic para abrir la tabla

## Table structure

structure-column-count = { $count ->
    [one] — 1 columna
   *[other] — { $count } columnas
}
structure-no-columns = (sin columnas)
structure-col-column = Columna
structure-col-type = Tipo
structure-col-key = Clave

## Data grid

grid-sort-asc-hint = Clic: ordenar ↑
grid-sort-desc-hint = Clic: ordenar ↓
grid-sort-off-hint = Clic: quitar ordenación
grid-remove-new-row = Quitar la fila nueva
grid-running-query = Ejecutando consulta…
grid-page-empty = — ninguna fila en esta página
grid-page-rows = — filas { $first }–{ $last }
grid-page-size-hint = Se piden { $count } filas por página (se cambia en Ajustes).
grid-page-previous = Anterior
grid-page-previous-hint = Página anterior
grid-page-next = Siguiente
grid-page-next-hint = Página siguiente
grid-row-count = { $count ->
    [one] — 1 fila
   *[other] — { $count } filas
}
grid-view-read-only = Vista · solo lectura
grid-view-read-only-hint =
    Los datos de una vista son los de sus tablas: se editan en ellas.
    Su definición se abre desde el menú de la vista en el árbol.
grid-row = Fila
grid-insert-row-hint = Insertar una fila
grid-no-primary-key = Sin clave primaria
grid-no-primary-key-hint =
    Sin clave primaria no hay forma segura de identificar una fila:
    las que ya existen no se pueden editar ni eliminar desde aquí.
    Sí se pueden añadir filas nuevas.
grid-sort = Orden:
grid-sort-remove = Quitar «{ $column }»
grid-sort-clear = Limpiar todo
grid-sort-clear-hint = Eliminar toda la ordenación

## Grid context menu

menu-copy-value = Copiar valor
menu-copy-csv = Copiar en CSV
menu-copy-tsv = Copiar en TSV
menu-copy-inserts = Copiar SQL Insert
menu-edit-cell = Editar celda
menu-set-null = Establecer NULL
menu-set-empty = Establecer vacío
menu-now = Fecha/hora actual
menu-default-value = Valor por defecto
menu-undo-change = Deshacer el cambio
menu-filter-clipboard = Por portapapeles
menu-filter-text = Por texto...
menu-insert-row = Insertar fila
menu-clone-row = Clonar fila
menu-undelete = Desmarcar eliminación
menu-delete-rows = { $count ->
    [one] Eliminar fila
   *[other] Eliminar { $count } filas
}

## Filter

filter-history-hint = Historial de filtros recientes
filter-no-history = Sin historial
filter-hint = id = 1 AND nombre LIKE 'foo%'
filter-field-hint = SQL directo — se inyecta como cláusula WHERE en la consulta
filter-apply = Aplicar
filter-clear = Limpiar filtro
filter-by-title = Filtrar por { $column }
filter-operator = Operador:
filter-value = Valor:
filter-value-hint = valor a filtrar
filter-op-equals = igual a
filter-op-not-equals = distinto de
filter-op-contains = contiene
filter-op-starts-with = empieza por
filter-op-ends-with = termina en
filter-op-greater = mayor que
filter-op-less = menor que
filter-op-greater-or-equal = mayor o igual
filter-op-less-or-equal = menor o igual

## Pending changes and saving

apply-no-primary-key = Sin clave primaria — no se puede guardar
apply-edits = { $count ->
    [one] 1 edición
   *[other] { $count } ediciones
}
apply-deletes = { $count ->
    [one] 1 eliminación
   *[other] { $count } eliminaciones
}
apply-inserts = { $count ->
    [one] 1 inserción
   *[other] { $count } inserciones
}
apply-discard = Descartar
apply-save = Guardar
save-no-open-table = No hay ninguna tabla abierta.
save-no-data = No hay datos cargados.
save-structure-loading = Aún se está leyendo la estructura de la tabla: inténtalo de nuevo.
save-no-primary-key = La tabla no tiene clave primaria: no se pueden editar ni eliminar filas.
save-no-column = No existe la columna { $index }.
save-cannot-title = No se pueden guardar los cambios
save-failed-title = Error al guardar
log-save-changes = Guardar cambios: { $deletes } eliminación(es), { $edits } edición(es), { $inserts } inserción(es) (transacción)
changes-saved = { $count ->
    [one] 1 cambio guardado en «{ $table }»
   *[other] { $count } cambios guardados en «{ $table }»
}
save-row-key-missing = No se encuentra la clave primaria de la fila { $row }.
save-row-not-found = No se ha guardado nada: una sentencia debía cambiar una fila y encontró { $found }. La fila puede haber cambiado o desaparecido en el servidor; recarga la tabla (F5) y repite los cambios.

## SQL editor

sql-tab-title = SQL { $number }
sql-database = BD:
sql-no-database = (ninguna)
sql-free-query = SQL libre:
sql-run-selection = Ejecutar selección
sql-run = Ejecutar
sql-clear = Limpiar
sql-cancelling = Cancelando…
sql-running = Ejecutando…
sql-cancel-hint = Interrumpe la consulta en el servidor
sql-editor-hint = SELECT * FROM tabla LIMIT 100
sql-empty = Escribe una consulta y pulsa { $shortcut } o el botón Ejecutar
sql-cancelled = Consulta cancelada
sql-done-no-result = Ejecutada, sin resultado que mostrar
sql-result-summary = { $rows ->
    [one] 1 fila
   *[other] { $rows } filas
}, { $columns ->
    [one] 1 columna
   *[other] { $columns } columnas
}
sql-result-truncated = solo las primeras { $count } (hay más; el tope se cambia en Ajustes)

## Query log

log-title = Log de consultas
log-entry-count = { $count ->
    [one] 1 entrada
   *[other] { $count } entradas
}
log-cancelled = CANCELADA
log-rows = { $count ->
    [one] 1 fila
   *[other] { $count } filas
}
log-copy-query = Copiar la consulta
log-copy-error = Copiar el error
log-open-in-sql-tab = Abrir en una pestaña SQL
log-error = ERROR
log-cancel-query = Cancelar la consulta
log-export = Exportación
log-export-finished = { $count ->
    [one] Exportación finalizada: 1 tabla
   *[other] Exportación finalizada: { $count } tablas
}

## Create, drop and empty dialogs

discard-sql-close-title = Cerrar «{ $title }»
discard-sql-close = Cerrar pestaña
discard-sql-clear-title = Limpiar «{ $title }»
discard-sql-warning = La consulta de esta pestaña se perderá:
create = Crear
create-name = Nombre:
create-database-name-hint = mi_base_de_datos
create-table-name = Nombre de tabla:
create-table-name-hint = mi_tabla
create-columns = Columnas:
create-col-name = Nombre
create-col-length = Longitud
create-add-column = Añadir columna
drop-confirm-database = ¿Seguro que quieres eliminar la base de datos:
drop-confirm-view = ¿Seguro que quieres eliminar la vista:
drop-confirm-table = ¿Seguro que quieres eliminar la tabla:
drop-cannot-undo = Esta acción no se puede deshacer.
drop-type-name = Escribe su nombre para confirmar:
truncate-title = Vaciar datos
truncate-confirm-all = ¿Seguro que quieres vaciar todas las tablas de:
truncate-confirm-table = ¿Seguro que quieres vaciar la tabla:
truncate-warning = Se eliminarán todos los datos. No se puede deshacer.
truncate-button = Vaciar
unsaved-title = Cambios sin guardar
unsaved-changes = { $count ->
    [one] Tienes 1 cambio sin guardar en «{ $table }».
   *[other] Tienes { $count } cambios sin guardar en «{ $table }».
}
unsaved-switch-question = ¿Qué quieres hacer antes de abrir «{ $table }»?
unsaved-save-and-switch = Guardar y cambiar
unsaved-discard-and-switch = Descartar y cambiar
unsaved-lost-on-page = Al cambiar de página se perderían.
unsaved-lost-on-reload = Al recargar los datos se perderían.
unsaved-save-and-reload = Guardar y recargar
unsaved-discard-and-reload = Descartar y recargar
unsaved-lost-on-close = Si cierras la pestaña se perderán.
unsaved-discard-and-close = Descartar y cerrar
clone-title = Clonar tabla — { $table }
clone-new-name = Nuevo nombre:
clone-name-hint = { $table }_copia
clone-copy-data = Copiar datos:
clone-button = Clonar

## Favourites

favourite-deleted = Favorito «{ $name }» eliminado
favourite-saved = Favorito «{ $name }» guardado
favourite-name-missing = Pon un nombre al favorito para guardarlo.
favourite-name-taken = Ya hay un favorito llamado «{ $name }»: cambia el nombre para guardarlo como otro.
favourite-delete-title = Eliminar favorito
favourite-delete-question = ¿Seguro que quieres eliminar el favorito:
favourite-delete-keyring = Se borrará también su contraseña del llavero.

## Notifications

database-list-failed = No se pudo leer la lista de bases de datos.
clipboard-read-failed = No se pudo leer el portapapeles: { $error }
name-copied = «{ $name }» copiado al portapapeles
rows-copied = { $count ->
    [one] 1 fila copiada ({ $format })
   *[other] { $count } filas copiadas ({ $format })
}
database-created = Base de datos «{ $db }» creada
database-dropped = Base de datos «{ $db }» eliminada
database-drop-failed = No se pudo eliminar «{ $db }»
table-created = Tabla «{ $table }» creada
table-dropped = «{ $table }» eliminada
table-drop-failed = No se pudo eliminar «{ $table }»
tables-list-failed = No se pudieron listar las tablas de «{ $db }»
tables-empty-failed = No se pudieron vaciar todas las tablas de «{ $db }»
table-emptied = Tabla «{ $table }» vaciada
tables-emptied = { $count ->
    [one] 1 tabla de «{ $db }» vaciada
   *[other] { $count } tablas de «{ $db }» vaciadas
}

## Export dialog

export-title-table = Exportar «{ $table }»
export-thread-died = El hilo de exportación terminó de forma inesperada.
export-title = Exportar datos — { $db }
export-tables-heading = Tablas ({ $selected }/{ $total })
export-select-all = Todo
export-select-none = Ninguno
export-destination = Destino
export-destination-label = Destino:
export-to-file = Fichero SQL (.sql)
export-to-folder = Carpeta (un .sql por tabla)
export-to-database = Otra base de datos (mismo servidor)
export-to-server = Otro servidor (favoritos)
export-choose = Seleccionar…
export-no-profiles = Sin perfiles
export-no-profiles-hint = Guarda antes un perfil con el servidor destino.
export-target-database = BBDD destino:
export-options = Opciones
export-include-create = Incluir CREATE TABLE
export-include-data = Incluir datos (INSERT INTO)
export-button = Exportar
export-select-a-table = Selecciona al menos una tabla
export-progress = { $done }/{ $total } tablas
export-tables-done = Tablas completadas: { $done } / { $total }
export-rows-done = Filas exportadas:   { $rows }
export-rows-unit = f.
export-cancel = Cancelar exportación
export-completed = Completado
export-completed-errors = { $count ->
    [one] Completado con 1 error
   *[other] Completado con { $count } errores
}
export-done = Exportación completada
export-done-errors = Exportación completada con errores
export-summary-tables = Tablas: { $count }
export-summary-errors = Con error: { $count } (detalle en el log)
export-summary-rows = Filas:  { $rows }
export-summary-time = Tiempo: { $seconds } s
export-cancelled = Cancelado
export-failed = La exportación falló:
export-same-target = El destino es la misma base de datos del mismo servidor que el origen: la copia borraría o duplicaría sus datos. Elige otra base de datos destino.
export-source-connection = Conexión al origen: { $error }
export-target-connection = Conexión al destino: { $error }
export-same-server-check-failed = No se pudo comprobar si el destino es el mismo servidor: { $error }
export-create-target-failed = No se pudo crear la BBDD destino «{ $db }»: { $error }
export-file-create-failed = No se pudo crear el fichero «{ $path }»: { $error }
export-file-write-failed = Error escribiendo fichero: { $error }
export-folder-create-failed = No se pudo crear la carpeta «{ $folder }»: { $error }
export-cancelled-error = Exportación cancelada.
export-drop-existing-failed =
    No se pudieron eliminar del destino las tablas que ya existen: { $error }

    Si otras tablas o vistas del destino dependen de ellas, expórtalas también o elimínalas antes.
export-no-ddl = No se obtuvo DDL para la tabla «{ $table }»
export-table-gone = La tabla «{ $table }» ya no existe.
export-foreign-table = Las tablas foráneas no se exportan.
export-dump-header = RustQL Export — Base de datos: { $db }
export-dump-generated = Generado: { $date }
export-dump-table = Tabla: { $table }
export-dump-deferred = Vistas y claves foráneas
export-partitioned-table = Las tablas particionadas no se exportan: sus particiones no se copiarían. Usa pg_dump.
export-postgres-scope = Se copian tablas y vistas con sus secuencias, claves e índices. No disparadores, funciones, permisos ni particiones: para una copia completa, usa pg_dump.

## Database errors

view-definition-failed = No se pudo leer la definición de «{ $view }»: { $error }
cancel-query-failed = No se pudo cancelar la consulta: { $error }
db-no-connection = Sin conexión con el servidor: { $error }
db-no-connection-to = Sin conexión con «{ $db }»: { $error }
db-query-error = Error en query: { $error }
db-write-error = Error en escritura: { $error }
db-begin-error = Error iniciando transacción: { $error }
db-transaction-error = Error en transacción (ROLLBACK): { $error }
db-commit-error = Error en COMMIT: { $error }
editor-session-failed = No se pudo abrir la sesión del editor: { $error }
view-definition-missing = No se encuentra la definición de la vista «{ $view }».
view-materialized-comment =
    -- Una vista materializada no se puede reemplazar: para cambiarla hay
    -- que eliminarla y volver a crearla.
charset-invalid = Charset inválido: «{ $charset }». Solo se permiten caracteres alfanuméricos y «_».
