//! Export from PostgreSQL. There is no `SHOW CREATE TABLE`: the structure of
//! each table is rebuilt from the catalog, with the functions that already
//! return its default values, keys, indexes and views written out.
//!
//! Tables (with their sequences, keys, indexes and the enum types of their
//! columns) and views are exported. Not triggers, functions, privileges,
//! policies or partitions: for a full copy, use `pg_dump`.
//!
//! Foreign keys are created at the end, once every table is loaded: there
//! is no way to turn their checks off as in MySQL.

use std::collections::HashMap;

use super::{
    ExportDestination, ExportOptions, Job, RowBatch, SAME_TARGET_ERROR, Sink, Source, TableData,
    TableScript, Totals, export_into, export_to_file, export_to_folder,
};
use crate::db::dialect::{PG_DEFAULT_SCHEMA, pg_split_table};
use crate::db::postgres::{Dedicated, PostgresBackend};
use crate::db::{ConnectionParams, DbBackend, DbResult, Engine};

const SQL: Engine = Engine::Postgres;

pub(super) fn run(
    params: &ConnectionParams,
    destination: &ExportDestination,
    job: &Job,
) -> DbResult<Totals> {
    if let ExportDestination::SameServerDb { target_db } = destination
        && target_db == job.source_db
    {
        return Err(SAME_TARGET_ERROR.into());
    }
    let backend = PostgresBackend::connect_with_progress(params, |_| {})
        .map_err(|e| format!("Conexión al origen: {e}"))?;
    let source = PgSource::open(&backend, job.source_db)?;
    match destination {
        ExportDestination::SqlFile { path } => export_to_file(&source, path, job),
        ExportDestination::Folder { path } => export_to_folder(&source, path, job),
        // Another database is another connection, on the same server or not
        ExportDestination::SameServerDb { target_db } => {
            export_into(&source, &mut PgSink::open(&backend, target_db)?, job)
        }
        ExportDestination::RemoteServer { params, target_db } => {
            let target = PostgresBackend::connect_with_progress(params, |_| {})
                .map_err(|e| format!("Conexión al destino: {e}"))?;
            // Another profile (or another host name) may lead to the same server:
            // copying a database onto itself drops (DROP) or duplicates its data.
            if target_db == job.source_db
                && same_server(&backend, &target).map_err(|e| {
                    format!("No se pudo comprobar si el destino es el mismo servidor: {e}")
                })?
            {
                return Err(SAME_TARGET_ERROR.into());
            }
            export_into(&source, &mut PgSink::open(&target, target_db)?, job)
        }
    }
}

/// Whether two connections reach the same server, according to the
/// identifier each cluster gets when it is created.
fn same_server(a: &PostgresBackend, b: &PostgresBackend) -> DbResult<bool> {
    let identity = |backend: &PostgresBackend| {
        backend
            .execute_query(None, "SELECT system_identifier FROM pg_control_system()")
            .map(|r| r.rows.into_iter().next())
    };
    let (id_a, id_b) = (identity(a)?, identity(b)?);
    Ok(id_a.is_some() && id_a == id_b)
}

/// Connection to the destination database (created if it does not exist).
struct PgSink<'a>(Dedicated<'a>);

impl<'a> PgSink<'a> {
    fn open(backend: &'a PostgresBackend, target_db: &str) -> DbResult<Self> {
        backend
            .ensure_database(target_db)
            .map_err(|e| format!("No se pudo crear la BBDD destino '{target_db}': {e}"))?;
        backend.dedicated(target_db).map(Self)
    }
}

impl Sink for PgSink<'_> {
    fn run(&mut self, sql: &str) -> DbResult<()> {
        self.0.execute(sql)
    }
}

/// A table or view of the source.
struct Relation {
    oid: String,
    /// `r` table, `p` partitioned, `v` view, `m` materialized view,
    /// `f` foreign table.
    kind: String,
    unlogged: bool,
    schema: String,
    /// `"schema"."table"`.
    reference: String,
}

/// Sequence of a column: owned by the table (`serial`) or internal to an
/// identity column.
struct Sequence {
    /// With its schema, and quoted if it needs it.
    name: String,
    data_type: String,
    /// `START WITH … INCREMENT BY …` and the rest of its parameters.
    options: String,
    owned: bool,
}

struct PgSource<'a> {
    /// With an empty `search_path`: that way the catalog writes every name
    /// with its schema and the statements work in any session.
    session: Dedicated<'a>,
}

fn cell(row: &[Option<String>], i: usize) -> &str {
    row.get(i).and_then(|v| v.as_deref()).unwrap_or("")
}

impl<'a> PgSource<'a> {
    fn open(backend: &'a PostgresBackend, db: &str) -> DbResult<Self> {
        let session = backend.dedicated(db)?;
        session.execute("SET search_path = ''")?;
        Ok(Self { session })
    }

    fn relation(&self, table: &str) -> DbResult<Relation> {
        let (schema, name) = pg_split_table(table);
        let sql = format!(
            "SELECT c.oid, c.relkind, c.relpersistence = 'u' \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = {} AND c.relname = {}",
            SQL.literal(&schema),
            SQL.literal(name)
        );
        let result = self.session.query(&sql)?;
        let row = result
            .rows
            .first()
            .ok_or_else(|| format!("La tabla «{table}» ya no existe."))?;
        Ok(Relation {
            oid: cell(row, 0).to_string(),
            kind: cell(row, 1).to_string(),
            unlogged: cell(row, 2) == "t",
            schema: schema.into_owned(),
            reference: SQL.table_ref(table),
        })
    }

    fn create_schema(&self, rel: &Relation) -> Option<String> {
        (rel.schema != PG_DEFAULT_SCHEMA).then(|| {
            format!(
                "CREATE SCHEMA IF NOT EXISTS {}",
                SQL.quote_ident(&rel.schema)
            )
        })
    }

    /// A view is created at the end: it depends on tables (and on other views).
    fn view_script(&self, rel: &Relation, options: &ExportOptions) -> DbResult<TableScript> {
        let mut script = TableScript::default();
        if !options.include_create {
            return Ok(script);
        }
        let sql = format!("SELECT pg_get_viewdef({}::oid, true)", rel.oid);
        let result = self.session.query(&sql)?;
        let definition = result.rows.first().map(|r| cell(r, 0)).unwrap_or_default();
        let definition = definition.trim().trim_end_matches(';');
        let kind = if rel.kind == "m" {
            "MATERIALIZED VIEW"
        } else {
            "VIEW"
        };
        script.finish.extend(self.create_schema(rel));
        script
            .finish
            .push(format!("CREATE {kind} {} AS\n{definition}", rel.reference));
        Ok(script)
    }

    /// Sequences of the table's columns, by column.
    fn sequences(&self, rel: &Relation) -> DbResult<HashMap<String, Sequence>> {
        let sql = format!(
            "SELECT a.attname, s.seqrelid::regclass::text, format_type(s.seqtypid, NULL), \
                    s.seqstart, s.seqincrement, s.seqmin, s.seqmax, s.seqcache, s.seqcycle, \
                    d.deptype \
             FROM pg_depend d \
             JOIN pg_sequence s ON s.seqrelid = d.objid \
             JOIN pg_attribute a ON a.attrelid = d.refobjid AND a.attnum = d.refobjsubid \
             WHERE d.classid = 'pg_class'::regclass AND d.refclassid = 'pg_class'::regclass \
               AND d.refobjid = {}::oid AND d.deptype IN ('a', 'i')",
            rel.oid
        );
        let result = self.session.query(&sql)?;
        Ok(result
            .rows
            .iter()
            .map(|row| {
                let cycle = if cell(row, 8) == "t" { " CYCLE" } else { "" };
                let sequence = Sequence {
                    name: cell(row, 1).to_string(),
                    data_type: cell(row, 2).to_string(),
                    options: format!(
                        "START WITH {} INCREMENT BY {} MINVALUE {} MAXVALUE {} CACHE {}{cycle}",
                        cell(row, 3),
                        cell(row, 4),
                        cell(row, 5),
                        cell(row, 6),
                        cell(row, 7)
                    ),
                    owned: cell(row, 9) == "a",
                };
                (cell(row, 0).to_string(), sequence)
            })
            .collect())
    }

    /// Enum types of the columns (arrays too), as statements that create them
    /// if they do not exist yet.
    fn enum_types(&self, rel: &Relation) -> DbResult<Vec<String>> {
        let sql = format!(
            "SELECT DISTINCT quote_ident(n.nspname), format_type(e.oid, NULL), \
                    (SELECT string_agg(quote_literal(l.enumlabel), ', ' ORDER BY l.enumsortorder) \
                     FROM pg_enum l WHERE l.enumtypid = e.oid) \
             FROM pg_attribute a \
             JOIN pg_type t ON t.oid = a.atttypid \
             JOIN pg_type e ON e.oid = CASE WHEN t.typcategory = 'A' THEN t.typelem ELSE t.oid END \
             JOIN pg_namespace n ON n.oid = e.typnamespace \
             WHERE a.attrelid = {}::oid AND a.attnum > 0 AND NOT a.attisdropped \
               AND e.typtype = 'e' \
             ORDER BY 2",
            rel.oid
        );
        let result = self.session.query(&sql)?;
        let mut statements = Vec::new();
        for row in &result.rows {
            statements.push(format!("CREATE SCHEMA IF NOT EXISTS {}", cell(row, 0)));
            // There is no CREATE TYPE IF NOT EXISTS: several tables may use the
            // same type, and it may already exist at the destination
            statements.push(format!(
                "DO $rustql$ BEGIN\n  CREATE TYPE {} AS ENUM ({});\n\
                 EXCEPTION WHEN duplicate_object THEN NULL;\nEND $rustql$",
                cell(row, 1),
                cell(row, 2)
            ));
        }
        Ok(statements)
    }

    fn table_script(&self, rel: &Relation, options: &ExportOptions) -> DbResult<TableScript> {
        let table = &rel.reference;
        let sql = format!(
            "SELECT a.attname, format_type(a.atttypid, a.atttypmod), a.attnotnull, \
                    pg_get_expr(d.adbin, d.adrelid), a.attidentity, a.attgenerated, \
                    CASE WHEN a.attcollation <> t.typcollation \
                         THEN quote_ident(cn.nspname) || '.' || quote_ident(co.collname) END \
             FROM pg_attribute a \
             JOIN pg_type t ON t.oid = a.atttypid \
             LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
             LEFT JOIN pg_collation co ON co.oid = a.attcollation \
             LEFT JOIN pg_namespace cn ON cn.oid = co.collnamespace \
             WHERE a.attrelid = {}::oid AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY a.attnum",
            rel.oid
        );
        let columns = self.session.query(&sql)?.rows;
        let sequences = self.sequences(rel)?;
        let mut script = TableScript::default();

        if options.include_create {
            script.create.extend(self.create_schema(rel));
            script.create.extend(self.enum_types(rel)?);
            // The sequences of serial columns go before the table, which names
            // them in its default values
            for sequence in sequences.values().filter(|s| s.owned) {
                script.create.push(format!(
                    "CREATE SEQUENCE IF NOT EXISTS {} AS {} {}",
                    sequence.name, sequence.data_type, sequence.options
                ));
            }
            let definitions: Vec<String> = columns
                .iter()
                .map(|row| column_definition(row, sequences.get(cell(row, 0))))
                .collect();
            let unlogged = if rel.unlogged { "UNLOGGED " } else { "" };
            script.create.push(format!(
                "CREATE {unlogged}TABLE {table} (\n  {}\n)",
                definitions.join(",\n  ")
            ));
            for (column, sequence) in sequences.iter().filter(|(_, s)| s.owned) {
                script.create.push(format!(
                    "ALTER SEQUENCE {} OWNED BY {table}.{}",
                    sequence.name,
                    SQL.quote_ident(column)
                ));
            }

            // Keys and indexes after the data (loading with them is slower);
            // foreign keys once every table exists
            let sql = format!(
                "SELECT quote_ident(conname), contype, pg_get_constraintdef(oid) \
                 FROM pg_constraint \
                 WHERE conrelid = {}::oid AND contype IN ('p', 'u', 'c', 'x', 'f') \
                 ORDER BY contype <> 'p', conname",
                rel.oid
            );
            for row in &self.session.query(&sql)?.rows {
                let statement = format!(
                    "ALTER TABLE {table} ADD CONSTRAINT {} {}",
                    cell(row, 0),
                    cell(row, 2)
                );
                if cell(row, 1) == "f" {
                    script.finish.push(statement);
                } else {
                    script.after_data.push(statement);
                }
            }
            let sql = format!(
                "SELECT pg_get_indexdef(i.indexrelid) FROM pg_index i \
                 WHERE i.indrelid = {}::oid \
                   AND NOT EXISTS (SELECT 1 FROM pg_constraint c WHERE c.conindid = i.indexrelid) \
                 ORDER BY i.indexrelid",
                rel.oid
            );
            for row in &self.session.query(&sql)?.rows {
                script.after_data.push(cell(row, 0).to_string());
            }
        }

        // Generated columns are computed by the destination: not copied
        let copied: Vec<String> = columns
            .iter()
            .filter(|row| cell(row, 5).is_empty())
            .map(|row| SQL.quote_ident(cell(row, 0)))
            .collect();
        let always_identity = columns.iter().any(|row| cell(row, 4) == "a");
        script.data = (!copied.is_empty()).then(|| TableData {
            select: format!("SELECT {} FROM {table}", copied.join(", ")),
            insert_into: format!("INSERT INTO {table}"),
            // Without it, a GENERATED ALWAYS identity rejects its values
            values_keyword: if always_identity {
                "OVERRIDING SYSTEM VALUE VALUES"
            } else {
                "VALUES"
            },
        });

        // Each sequence carries on at the destination where it was at the
        // source. It is looked up by its column: the destination names an
        // identity's sequence
        if options.include_data {
            for (column, sequence) in &sequences {
                let state = self.session.query(&format!(
                    "SELECT last_value, is_called FROM {}",
                    sequence.name
                ))?;
                let Some(row) = state.rows.first() else {
                    continue;
                };
                script.after_data.push(format!(
                    "SELECT pg_catalog.setval(pg_catalog.pg_get_serial_sequence({}, {}), {}, {})",
                    SQL.literal(table),
                    SQL.literal(column),
                    cell(row, 0),
                    cell(row, 1) == "t"
                ));
            }
        }
        Ok(script)
    }
}

/// Definition of a column for `CREATE TABLE`, from its catalog row (see
/// `table_script`).
fn column_definition(row: &[Option<String>], sequence: Option<&Sequence>) -> String {
    let mut definition = format!("{} {}", SQL.quote_ident(cell(row, 0)), cell(row, 1));
    if let Some(collation) = row.get(6).and_then(|c| c.as_deref()) {
        definition.push_str(&format!(" COLLATE {collation}"));
    }
    let default = row.get(3).and_then(|d| d.as_deref());
    match (cell(row, 4), cell(row, 5), default) {
        // Generated: its "default value" is the expression that computes it
        (_, generated, Some(expression)) if !generated.is_empty() => {
            let storage = if generated == "v" {
                "VIRTUAL"
            } else {
                "STORED"
            };
            definition.push_str(&format!(" GENERATED ALWAYS AS ({expression}) {storage}"));
        }
        (identity @ ("a" | "d"), _, _) => {
            let when = if identity == "a" {
                "ALWAYS"
            } else {
                "BY DEFAULT"
            };
            definition.push_str(&format!(" GENERATED {when} AS IDENTITY"));
            if let Some(sequence) = sequence {
                definition.push_str(&format!(" ({})", sequence.options));
            }
        }
        (_, _, Some(expression)) => definition.push_str(&format!(" DEFAULT {expression}")),
        _ => {}
    }
    if cell(row, 2) == "t" {
        definition.push_str(" NOT NULL");
    }
    definition
}

impl Source for PgSource<'_> {
    fn engine(&self) -> Engine {
        SQL
    }

    fn prelude(&self) -> Vec<String> {
        vec![
            "SET client_encoding = 'UTF8'".into(),
            "SET standard_conforming_strings = on".into(),
        ]
    }

    /// In a single statement per kind: that way tables with foreign keys
    /// between them are dropped regardless of order. Views first.
    fn drop_together(&self, tables: &[String]) -> DbResult<Vec<String>> {
        let mut by_kind: [(&str, &str, Vec<String>); 3] = [
            ("v", "VIEW", Vec::new()),
            ("m", "MATERIALIZED VIEW", Vec::new()),
            ("", "TABLE", Vec::new()),
        ];
        for table in tables {
            // One that no longer exists will report its error when exported
            let Ok(rel) = self.relation(table) else {
                continue;
            };
            let group = by_kind
                .iter_mut()
                .find(|(kind, _, _)| *kind == rel.kind || kind.is_empty());
            if let Some((_, _, names)) = group {
                names.push(rel.reference);
            }
        }
        Ok(by_kind
            .into_iter()
            .filter(|(_, _, names)| !names.is_empty())
            .map(|(_, what, names)| format!("DROP {what} IF EXISTS {}", names.join(", ")))
            .collect())
    }

    fn script(&self, table: &str, options: &ExportOptions) -> DbResult<TableScript> {
        let rel = self.relation(table)?;
        match rel.kind.as_str() {
            "v" | "m" => self.view_script(&rel, options),
            "f" => Err("Las tablas foráneas no se exportan.".into()),
            "p" => Err(
                "Las tablas particionadas no se exportan: sus particiones no se copiarían. \
                 Usa pg_dump."
                    .into(),
            ),
            _ => self.table_script(&rel, options),
        }
    }

    fn read_rows(&self, select: &str, batch: usize, each: &mut RowBatch<'_>) -> DbResult<()> {
        self.session
            .read_batches(select, batch, &mut |columns, rows| {
                let literals = rows
                    .into_iter()
                    .map(|row| {
                        row.into_iter()
                            .map(|cell| cell.map_or("NULL".into(), |value| SQL.literal(&value)))
                            .collect()
                    })
                    .collect();
                each(columns, literals)
            })
    }
}

#[cfg(test)]
mod tests;
