# Changelog

All notable changes to RustQL are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/); 0.x versions are betas.

## [Unreleased]

First public beta.

### Added

- MySQL, MariaDB and PostgreSQL connections, several open at once in
  colour-coded tabs. A connection can be limited to a single database.
- Database tree with tables and views, favourites and a quick filter.
- Data grid with keyboard navigation, inline editing, new rows, paging for
  large tables and highlighting of cells that share the selected value. All
  pending changes are saved in a single transaction.
- SQL editor with syntax highlighting, several tabs, run selection,
  cancellation of running queries and a row limit for results.
- Query log: copy a query or reopen it in a SQL tab.
- Export databases or tables to a SQL file, a folder, another database on the
  same server or a remote server, for both MySQL and PostgreSQL.
- Encrypted connections (TLS) with certificate verification against the system
  authorities or a custom CA, and a padlock in the status bar.
- Passwords stored in the system keyring.
- Light and dark themes, selectable fonts and remembered window size.

[Unreleased]: https://github.com/soltik-dev/rustql/commits/master
