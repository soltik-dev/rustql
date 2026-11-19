# RustQL

A fast, native desktop client for MySQL, MariaDB and PostgreSQL, written in Rust.

> **Public beta.** RustQL 0.9 is the first public release. Expect rough edges,
> and please [open an issue](https://github.com/soltik-dev/rustql/issues) when
> something breaks. The interface is in Spanish for now; English, French,
> Italian and Portuguese are on the way.

## Features

- **MySQL, MariaDB and PostgreSQL.** Keep several connections open at once,
  each in its own colour-coded tab.
- **Browse and edit.** Databases, tables and views in a tree; edit rows in the
  grid and save every change in a single transaction.
- **SQL editor.** Syntax highlighting, run the whole script or just the
  selection, cancel a running query, results in the same grid.
- **Large tables.** Data is read page by page, and nothing blocks the window
  while the server works.
- **Export.** Databases or tables to a SQL file, a folder, another database or
  another server.
- **Secure by default.** Encrypted connections (TLS) with certificate
  verification. Passwords live in the system keyring (Secret Service, Windows
  Credential Manager, macOS Keychain), never in a plain-text file.
- **Light and dark themes.**

## Install

Builds for Windows, macOS and Linux (AppImage and a Fedora COPR repository)
will be published on [rustql.com](https://rustql.com) and on
[GitHub Releases](https://github.com/soltik-dev/rustql/releases) with the
first beta.

## Build from source

You need Rust 1.95 or later and a C compiler. On Linux, also `pkg-config` and
the zlib headers (`zlib-devel` on Fedora, `zlib1g-dev` on Debian and Ubuntu).

```sh
git clone https://github.com/soltik-dev/rustql.git
cd rustql
cargo build --release
./target/release/rustql
```

### Tests

```sh
cargo test
```

The integration tests need real servers and are skipped by default. Point them
at throwaway MySQL/MariaDB and PostgreSQL instances (they create and drop
their own databases) and run the ignored tests:

```sh
RUSTQL_TEST_MYSQL='user:password@127.0.0.1:3306' \
RUSTQL_TEST_POSTGRES='user:password@127.0.0.1:5432' \
  cargo test -- --ignored
```

The TLS tests also need `RUSTQL_TEST_CA`, the CA certificate that signed both
servers' certificates. [The CI workflow](.github/workflows/ci.yml) sets all of
this up from scratch and is the reference.

## License

Copyright © 2026 Alberto López

RustQL is free software: you can redistribute it and/or modify it under the
terms of the GNU General Public License as published by the Free Software
Foundation, either version 3 of the License, or (at your option) any later
version. See [LICENSE](LICENSE).

The bundled fonts (Inter, JetBrains Mono and the Font Awesome glyphs in
`assets/fonts`) are distributed under the SIL Open Font License 1.1; their
licenses are next to them.
