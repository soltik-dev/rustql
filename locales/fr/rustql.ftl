# RustQL — français.

## Shared

common-cancel = Annuler
common-error = Erreur : { $error }
common-loading = Chargement…
error-title = Erreur
toast-close-hint = Cliquez pour fermer
common-close = Fermer
common-delete = Supprimer

## Keys (as printed on the keyboard)

key-delete = Suppr
key-page-up = Pg préc
key-page-down = Pg suiv

## Status bar

status-schema = Schéma
status-log = Journal
status-settings = Réglages
status-disconnected = Non connecté
status-connecting = Connexion à { $host }…

## Settings panel

settings-title = Réglages
settings-language = Langue
settings-language-system = Système
settings-theme = Thème
settings-theme-system = Système
settings-theme-light = Clair
settings-theme-dark = Sombre
settings-font = Police
settings-font-embedded = Inter (intégrée)
settings-font-size = Taille de police
settings-font-sample = Texte d’exemple
settings-copy-format = Format de copie (plusieurs lignes)
settings-page-rows = Lignes par page (tables)
settings-editor-max-rows = Lignes max. dans l’éditeur SQL

## Connection tabs

conn-tabs-new = Nouvelle connexion
conn-tabs-close = Fermer la connexion

## Connection form

conn-favourites = Favoris
conn-no-favourites = Aucune connexion enregistrée.
conn-no-favourites-hint =
    Remplissez le formulaire
    et cliquez sur Enregistrer.
conn-subtitle = Se connecter à une base de données
conn-engine = Moteur :
conn-host = Hôte :
conn-port = Port :
conn-user = Utilisateur :
conn-password = Mot de passe :
conn-password-in-keyring = Enregistré dans le trousseau
conn-database = Base de données :
conn-database-all = Toutes
conn-database-pick = Choisir parmi les bases de données du serveur
conn-database-reading = Lecture des bases de données…
conn-security = Sécurité :
conn-ca-file = Fichier de CA :
conn-ca-pick = Choisir le fichier
conn-ca-filter-certificates = Certificats
conn-ca-filter-all = Tous
conn-ca-system = Celles du système
conn-ca-hint =
    Certificat (PEM) de l’autorité qui signe celui du serveur.
    Vide, les autorités installées sur le système sont utilisées.
conn-save-password = Enregistrer le mot de passe
conn-no-keyring = Pas de trousseau système : il ne sera pas enregistré
conn-save-password-hint = Conservé chiffré dans le trousseau du système
conn-profile-name = Nom :
conn-profile-name-hint = Mon serveur...
conn-color = Couleur :
conn-color-none = Sans couleur
conn-color-custom = Personnalisée…
conn-save = Enregistrer
conn-save-as-new = Enregistrer comme nouveau
conn-save-as-new-hint = Crée un autre favori avec ces données, sans toucher au favori sélectionné
conn-delete = Supprimer
conn-connect = Se connecter
conn-port-invalid = Port non valide : « { $port } ». Ce doit être un nombre entre 1 et 65535.
conn-host-missing = Indiquez l’hôte du serveur.

## Connection colours

color-blue = Bleu
color-teal = Turquoise
color-green = Vert
color-yellow = Jaune
color-orange = Orange
color-red = Rouge
color-pink = Rose
color-purple = Violet
color-custom = Personnalisée

## Connection security

tls-prefer = Chiffrer si possible
tls-require = Exiger le chiffrement
tls-verify = Vérifier le serveur
tls-prefer-description =
    Utilise TLS si le serveur le propose et, sinon, se connecte sans chiffrement.
    Ne vérifie pas le certificat.
tls-require-description =
    Ne se connecte pas si le serveur ne propose pas TLS.
    Ne vérifie pas le certificat : protège contre l’écoute du réseau,
    pas contre l’usurpation du serveur.
tls-verify-description =
    Ne se connecte pas sans TLS, et vérifie que le certificat appartient à ce
    serveur et qu’il est signé par une autorité de confiance du système
    ou par celle du fichier de CA indiqué.
security-verified = Connexion chiffrée (TLS), avec le certificat du serveur vérifié.
security-encrypted =
    Connexion chiffrée (TLS). Le certificat du serveur n’est pas vérifié :
    pour le vérifier, choisissez « { $verify } » dans la connexion.
security-local = Connexion non chiffrée, mais elle ne quitte pas cette machine.
security-plain =
    Connexion NON chiffrée : tout ce qui est envoyé et reçu, mot de passe compris,
    peut être lu sur le réseau. Le serveur ne propose pas TLS.
tls-hint-certificate = Le certificat du serveur n’a pas pu être vérifié. S’il est signé par une autorité privée, indiquez son fichier de CA ; si vous n’avez pas besoin de le vérifier, choisissez un autre niveau de sécurité.
tls-hint-required = La sécurité de cette connexion exige TLS. Si le serveur ne le propose pas, choisissez « { $prefer } ».
tls-ca-unreadable = Impossible de lire le fichier de CA « { $path } » : { $error }
tls-ca-empty = il ne contient aucun certificat au format PEM
tls-verifier-failed = Impossible de préparer la vérification du certificat : { $error }
tls-setup-failed = Impossible de préparer TLS : { $error }

## Connecting

keyring-unavailable = aucun trousseau système n’est disponible
keyring-locked = le trousseau est verrouillé ou le déverrouillage a été annulé ({ $error })
connect-thread-died = Le fil de connexion s’est arrêté de façon inattendue
connect-starting = Démarrage...
keyring-save-failed = Le mot de passe n’a pas été enregistré dans le trousseau : { $error }
connect-reading-keyring = Lecture du mot de passe dans le trousseau...
connect-tcp = Ouverture de la connexion TCP...
connect-authenticating = Authentification...
connect-connecting = Connexion au serveur...
connect-established = Connexion établie.
connect-setup-failed = Impossible de préparer la connexion : { $error }
connect-failed = Connexion impossible : { $error }{ $hint }
connect-hint-database = Si le serveur ne vous laisse entrer que dans votre propre base de données, indiquez-la dans le champ « Base de données ».
keyring-read-failed = Impossible de lire le mot de passe dans le trousseau : { $error }
keyring-password-gone = Le mot de passe de ce profil n’est plus dans le trousseau du système : saisissez-le et enregistrez de nouveau le profil.

## Session

session-connecting = Connexion à { $host }...
session-pick-database = Sélectionnez une base de données dans le panneau de gauche
session-pick-table = Sélectionnez une table dans le panneau de gauche

## Session tab bar

tab-database = Base
tab-structure = Structure
tab-structure-of = Structure de { $table }
tab-data = Données
tab-rename = Renommer
tab-close = Fermer
tab-close-sql-hint = Fermer l’onglet SQL  { $shortcut } (ou clic du milieu sur l’onglet)
tab-new-sql = Nouvel onglet SQL
tab-name-hint = Nom de l’onglet

## Schema tree

tree-title = Schéma
tree-showing-favourites = Favoris uniquement
tree-filter-favourites = Filtrer les favoris
tree-reload = Recharger le schéma
tree-search = Rechercher…
tree-create-database = Créer une base de données
tree-refresh = Actualiser
tree-disconnect = Se déconnecter
tree-no-matches = (aucun résultat)
tree-no-tables = (aucune table)
tree-create-table = Créer une table
tree-export-database = Exporter la base de données
tree-empty-all-tables = Vider toutes les tables
tree-drop-database = Supprimer la base de données
tree-unfavourite = Retirer des favoris
tree-favourite = Ajouter aux favoris
tree-show-structure = Voir la structure
tree-open-in-sql = Ouvrir en SQL
tree-copy-name = Copier le nom
tree-show-definition = Voir la définition
tree-export-view = Exporter la vue
tree-drop-view = Supprimer la vue
tree-clone-table = Cloner la table
tree-export-table = Exporter la table
tree-empty-table = Vider la table
tree-drop-table = Supprimer la table

## Server and database panels

server-info-title = Serveur : { $host }
server-info-database-count = { $count ->
    [one] — { $count } base de données
   *[other] — { $count } bases de données
}
server-info-no-databases = (aucune base de données)
server-info-col-database = Base de données
server-info-col-tables = Tables
server-info-col-size = Taille
db-info-title = Base de données : { $db }
db-info-table-count = { $count ->
    [one] — { $count } table
   *[other] — { $count } tables
}
db-info-no-tables = Cette base de données n’a aucune table.
db-info-col-table = Table
db-info-col-engine = Moteur
db-info-col-rows = Lignes
db-info-col-size = Taille
db-info-open-hint = Double-cliquez pour ouvrir la table

## Table structure

structure-column-count = { $count ->
    [one] — { $count } colonne
   *[other] — { $count } colonnes
}
structure-no-columns = (aucune colonne)
structure-col-column = Colonne
structure-col-type = Type
structure-col-key = Clé

## Data grid

grid-sort-asc-hint = Clic : trier ↑
grid-sort-desc-hint = Clic : trier ↓
grid-sort-off-hint = Clic : retirer le tri
grid-remove-new-row = Retirer la nouvelle ligne
grid-running-query = Exécution de la requête…
grid-page-empty = — aucune ligne sur cette page
grid-page-rows = — lignes { $first }–{ $last }
grid-page-size-hint = { $count } lignes sont demandées par page (à changer dans Réglages).
grid-page-previous = Précédente
grid-page-previous-hint = Page précédente
grid-page-next = Suivante
grid-page-next-hint = Page suivante
grid-row-count = { $count ->
    [one] — { $count } ligne
   *[other] — { $count } lignes
}
grid-view-read-only = Vue · lecture seule
grid-view-read-only-hint =
    Les données d’une vue sont celles de ses tables : modifiez-les là.
    Sa définition s’ouvre depuis le menu de la vue dans l’arbre.
grid-row = Ligne
grid-insert-row-hint = Insérer une ligne
grid-no-primary-key = Pas de clé primaire
grid-no-primary-key-hint =
    Sans clé primaire, il n’y a aucun moyen sûr d’identifier une ligne :
    les lignes existantes ne peuvent être ni modifiées ni supprimées ici.
    Il est toujours possible d’en ajouter de nouvelles.
grid-sort = Tri :
grid-sort-remove = Retirer « { $column } »
grid-sort-clear = Tout effacer
grid-sort-clear-hint = Retirer tout le tri

## Grid context menu

menu-copy-value = Copier la valeur
menu-copy-csv = Copier en CSV
menu-copy-tsv = Copier en TSV
menu-copy-inserts = Copier en SQL INSERT
menu-edit-cell = Modifier la cellule
menu-set-null = Mettre à NULL
menu-set-empty = Mettre à vide
menu-now = Date/heure actuelle
menu-default-value = Valeur par défaut
menu-undo-change = Annuler la modification
menu-filter-clipboard = Par le presse-papiers
menu-filter-text = Par texte...
menu-insert-row = Insérer une ligne
menu-clone-row = Cloner la ligne
menu-undelete = Annuler la suppression
menu-delete-rows = { $count ->
    [one] Supprimer la ligne
   *[other] Supprimer { $count } lignes
}

## Filter

filter-history-hint = Filtres récents
filter-no-history = Aucun historique
filter-hint = id = 1 AND nom LIKE 'foo%'
filter-field-hint = SQL brut — ajouté à la requête comme clause WHERE
filter-apply = Appliquer
filter-clear = Effacer le filtre
filter-by-title = Filtrer par { $column }
filter-operator = Opérateur :
filter-value = Valeur :
filter-value-hint = valeur à filtrer
filter-op-equals = égal à
filter-op-not-equals = différent de
filter-op-contains = contient
filter-op-starts-with = commence par
filter-op-ends-with = se termine par
filter-op-greater = supérieur à
filter-op-less = inférieur à
filter-op-greater-or-equal = supérieur ou égal
filter-op-less-or-equal = inférieur ou égal

## Pending changes and saving

apply-no-primary-key = Pas de clé primaire — impossible d’enregistrer
apply-edits = { $count ->
    [one] { $count } modification
   *[other] { $count } modifications
}
apply-deletes = { $count ->
    [one] { $count } suppression
   *[other] { $count } suppressions
}
apply-inserts = { $count ->
    [one] { $count } insertion
   *[other] { $count } insertions
}
apply-discard = Abandonner
apply-save = Enregistrer
save-no-open-table = Aucune table n’est ouverte.
save-no-data = Aucune donnée n’est chargée.
save-structure-loading = La structure de la table est encore en cours de lecture : réessayez.
save-no-primary-key = La table n’a pas de clé primaire : les lignes ne peuvent être ni modifiées ni supprimées.
save-no-column = La colonne { $index } n’existe pas.
save-cannot-title = Impossible d’enregistrer les modifications
save-failed-title = Erreur lors de l’enregistrement
log-save-changes = Enregistrer les modifications : { $deletes } suppression(s), { $edits } modification(s), { $inserts } insertion(s) (transaction)
changes-saved = { $count ->
    [one] { $count } modification enregistrée dans « { $table } »
   *[other] { $count } modifications enregistrées dans « { $table } »
}
save-row-key-missing = Impossible de trouver la clé primaire de la ligne { $row }.
save-row-not-found = Rien n’a été enregistré : une instruction devait modifier une ligne et en a trouvé { $found }. La ligne a peut-être changé ou disparu sur le serveur ; rechargez la table (F5) et refaites les modifications.

## SQL editor

sql-tab-title = SQL { $number }
sql-database = Base :
sql-no-database = (aucune)
sql-free-query = SQL libre :
sql-run-selection = Exécuter la sélection
sql-run = Exécuter
sql-clear = Effacer
sql-cancelling = Annulation…
sql-running = Exécution…
sql-cancel-hint = Interrompt la requête sur le serveur
sql-editor-hint = SELECT * FROM table LIMIT 100
sql-empty = Écrivez une requête et appuyez sur { $shortcut } ou sur le bouton Exécuter
sql-cancelled = Requête annulée
sql-done-no-result = Exécutée, aucun résultat à afficher
sql-result-summary = { $rows ->
    [one] { $rows } ligne
   *[other] { $rows } lignes
}, { $columns ->
    [one] { $columns } colonne
   *[other] { $columns } colonnes
}
sql-result-truncated = seulement les { $count } premières (il y en a plus ; la limite se règle dans Réglages)

## Query log

log-title = Journal des requêtes
log-entry-count = { $count ->
    [one] { $count } entrée
   *[other] { $count } entrées
}
log-cancelled = ANNULÉE
log-rows = { $count ->
    [one] { $count } ligne
   *[other] { $count } lignes
}
log-copy-query = Copier la requête
log-copy-error = Copier l’erreur
log-open-in-sql-tab = Ouvrir dans un onglet SQL
log-error = ERREUR
log-cancel-query = Annuler la requête
log-export = Export
log-export-finished = { $count ->
    [one] Export terminé : { $count } table
   *[other] Export terminé : { $count } tables
}

## Create, drop and empty dialogs

discard-sql-close-title = Fermer « { $title } »
discard-sql-close = Fermer l’onglet
discard-sql-clear-title = Effacer « { $title } »
discard-sql-warning = La requête de cet onglet sera perdue :
create = Créer
create-name = Nom :
create-database-name-hint = ma_base
create-table-name = Nom de la table :
create-table-name-hint = ma_table
create-columns = Colonnes :
create-col-name = Nom
create-col-length = Longueur
create-add-column = Ajouter une colonne
drop-confirm-database = Voulez-vous vraiment supprimer la base de données :
drop-confirm-view = Voulez-vous vraiment supprimer la vue :
drop-confirm-table = Voulez-vous vraiment supprimer la table :
drop-cannot-undo = Cette action est irréversible.
drop-type-name = Saisissez son nom pour confirmer :
truncate-title = Vider les données
truncate-confirm-all = Voulez-vous vraiment vider toutes les tables de :
truncate-confirm-table = Voulez-vous vraiment vider la table :
truncate-warning = Toutes les données seront supprimées. Cette action est irréversible.
truncate-button = Vider
unsaved-title = Modifications non enregistrées
unsaved-changes = { $count ->
    [one] Vous avez { $count } modification non enregistrée dans « { $table } ».
   *[other] Vous avez { $count } modifications non enregistrées dans « { $table } ».
}
unsaved-switch-question = Que voulez-vous faire avant d’ouvrir « { $table } » ?
unsaved-save-and-switch = Enregistrer et changer
unsaved-discard-and-switch = Abandonner et changer
unsaved-lost-on-page = Changer de page les ferait perdre.
unsaved-lost-on-reload = Recharger les données les ferait perdre.
unsaved-save-and-reload = Enregistrer et recharger
unsaved-discard-and-reload = Abandonner et recharger
unsaved-lost-on-close = Fermer l’onglet les fera perdre.
unsaved-discard-and-close = Abandonner et fermer
clone-title = Cloner la table — { $table }
clone-new-name = Nouveau nom :
clone-name-hint = { $table }_copie
clone-copy-data = Copier les données :
clone-button = Cloner

## Favourites

favourite-deleted = Favori « { $name } » supprimé
favourite-saved = Favori « { $name } » enregistré
favourite-name-missing = Donnez un nom au favori pour l’enregistrer.
favourite-name-taken = Il existe déjà un favori nommé « { $name } » : changez le nom pour l’enregistrer comme un autre.
favourite-delete-title = Supprimer le favori
favourite-delete-question = Voulez-vous vraiment supprimer le favori :
favourite-delete-keyring = Son mot de passe sera aussi retiré du trousseau.

## Notifications

database-list-failed = Impossible de lire la liste des bases de données.
clipboard-read-failed = Impossible de lire le presse-papiers : { $error }
name-copied = « { $name } » copié dans le presse-papiers
rows-copied = { $count ->
    [one] { $count } ligne copiée ({ $format })
   *[other] { $count } lignes copiées ({ $format })
}
database-created = Base de données « { $db } » créée
database-dropped = Base de données « { $db } » supprimée
database-drop-failed = Impossible de supprimer « { $db } »
table-created = Table « { $table } » créée
table-dropped = « { $table } » supprimée
table-drop-failed = Impossible de supprimer « { $table } »
tables-list-failed = Impossible de lister les tables de « { $db } »
tables-empty-failed = Impossible de vider toutes les tables de « { $db } »
table-emptied = Table « { $table } » vidée
tables-emptied = { $count ->
    [one] { $count } table de « { $db } » vidée
   *[other] { $count } tables de « { $db } » vidées
}

## Export dialog

export-title-table = Exporter « { $table } »
export-thread-died = Le fil d’export s’est arrêté de façon inattendue.
export-title = Exporter les données — { $db }
export-tables-heading = Tables ({ $selected }/{ $total })
export-select-all = Toutes
export-select-none = Aucune
export-destination = Destination
export-destination-label = Destination :
export-to-file = Fichier SQL (.sql)
export-to-folder = Dossier (un .sql par table)
export-to-database = Une autre base de données (même serveur)
export-to-server = Un autre serveur (favoris)
export-choose = Choisir…
export-no-profiles = Aucun profil
export-no-profiles-hint = Enregistrez d’abord un profil avec le serveur de destination.
export-target-database = Base de destination :
export-options = Options
export-include-create = Inclure CREATE TABLE
export-include-data = Inclure les données (INSERT INTO)
export-button = Exporter
export-select-a-table = Sélectionnez au moins une table
export-progress = { $done }/{ $total } tables
export-tables-done = Tables terminées : { $done } / { $total }
export-rows-done = Lignes exportées :   { $rows }
export-rows-unit = lignes
export-cancel = Annuler l’export
export-completed = Terminé
export-completed-errors = { $count ->
    [one] Terminé avec { $count } erreur
   *[other] Terminé avec { $count } erreurs
}
export-done = Export terminé
export-done-errors = Export terminé avec des erreurs
export-summary-tables = Tables : { $count }
export-summary-errors = En erreur : { $count } (détails dans le journal)
export-summary-rows = Lignes :  { $rows }
export-summary-time = Durée : { $seconds } s
export-cancelled = Annulé
export-failed = L’export a échoué :
export-same-target = La destination est la même base de données du même serveur que la source : la copie supprimerait ou dupliquerait ses données. Choisissez une autre base de destination.
export-source-connection = Connexion à la source : { $error }
export-target-connection = Connexion à la destination : { $error }
export-same-server-check-failed = Impossible de vérifier si la destination est le même serveur : { $error }
export-create-target-failed = Impossible de créer la base de destination « { $db } » : { $error }
export-file-create-failed = Impossible de créer le fichier « { $path } » : { $error }
export-file-write-failed = Erreur d’écriture du fichier : { $error }
export-folder-create-failed = Impossible de créer le dossier « { $folder } » : { $error }
export-cancelled-error = Export annulé.
export-drop-existing-failed =
    Impossible de supprimer les tables qui existent déjà à la destination : { $error }

    Si d’autres tables ou vues de la destination en dépendent, exportez-les aussi ou supprimez-les d’abord.
export-no-ddl = Aucun DDL obtenu pour la table « { $table } »
export-table-gone = La table « { $table } » n’existe plus.
export-foreign-table = Les tables étrangères ne sont pas exportées.
export-dump-header = Export RustQL — Base de données : { $db }
export-dump-generated = Généré le : { $date }
export-dump-table = Table : { $table }
export-dump-deferred = Vues et clés étrangères
export-partitioned-table = Les tables partitionnées ne sont pas exportées : leurs partitions ne seraient pas copiées. Utilisez pg_dump.
export-postgres-scope = Les tables et les vues sont copiées avec leurs séquences, clés et index. Pas les déclencheurs, fonctions, privilèges ni partitions : pour une copie complète, utilisez pg_dump.

## Database errors

view-definition-failed = Impossible de lire la définition de « { $view } » : { $error }
cancel-query-failed = Impossible d’annuler la requête : { $error }
db-no-connection = Pas de connexion au serveur : { $error }
db-no-connection-to = Pas de connexion à « { $db } » : { $error }
db-query-error = Erreur de requête : { $error }
db-write-error = Erreur d’écriture : { $error }
db-begin-error = Impossible de démarrer la transaction : { $error }
db-transaction-error = Erreur dans la transaction (ROLLBACK) : { $error }
db-commit-error = Erreur lors du COMMIT : { $error }
editor-session-failed = Impossible d’ouvrir la session de l’éditeur : { $error }
view-definition-missing = Impossible de trouver la définition de la vue « { $view } ».
view-materialized-comment =
    -- Une vue matérialisée ne peut pas être remplacée : pour la modifier,
    -- il faut la supprimer et la recréer.
charset-invalid = Jeu de caractères non valide : « { $charset } ». Seuls les caractères alphanumériques et « _ » sont autorisés.
