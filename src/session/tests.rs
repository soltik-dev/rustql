use super::schema::SchemaResult;
use super::view::FrameSignals;
use super::writes::AfterCommit;
use super::*;
use crate::db::{CancelFlag, ColumnInfo, DatabaseInfo, DbResult, QUERY_CANCELLED, TableInfo};
use crate::ui::connection_panel::SaveProfile;
use crate::ui::modals::NewColumnDef;
use crate::ui::sql_editor::SqlEditorAction;

/// Backend sin servidor: las lecturas devuelven vacío y las escrituras, ok.
struct FakeBackend;

impl DbBackend for FakeBackend {
    fn list_databases(&self) -> DbResult<Vec<String>> {
        Ok(Vec::new())
    }
    fn list_tables(&self, _: &str) -> DbResult<Vec<String>> {
        Ok(Vec::new())
    }
    fn execute_query(&self, _: &str) -> DbResult<QueryResult> {
        Ok(QueryResult {
            columns: Vec::new(),
            rows: Vec::new(),
            ..Default::default()
        })
    }
    fn execute_in_session(
        &self,
        _: Option<&str>,
        sql: &str,
        _: usize,
        cancel: &CancelFlag,
    ) -> DbResult<QueryResult> {
        // «LENTA» no termina hasta que la cancelan
        let start = Instant::now();
        while sql == "LENTA" && !cancel.load(Ordering::SeqCst) {
            assert!(start.elapsed() < Duration::from_secs(10), "sin cancelar");
            std::thread::sleep(Duration::from_millis(1));
        }
        if cancel.load(Ordering::SeqCst) {
            return Err(QUERY_CANCELLED.into());
        }
        self.execute_query(sql)
    }
    fn cancel_in_session(&self, cancel: &CancelFlag) -> DbResult<()> {
        cancel.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn get_databases_info(&self) -> DbResult<Vec<DatabaseInfo>> {
        Ok(Vec::new())
    }
    fn get_tables_info(&self, _: &str) -> DbResult<Vec<TableInfo>> {
        Ok(Vec::new())
    }
    fn get_columns_info(&self, _: &str, _: &str) -> DbResult<Vec<ColumnInfo>> {
        Ok(Vec::new())
    }
    fn list_all_tables(&self) -> DbResult<Vec<(String, String)>> {
        Ok(Vec::new())
    }
    fn execute_write(&self, _: &str) -> DbResult<u64> {
        Ok(0)
    }
    fn create_database(&self, _: &str, _: &str) -> DbResult<()> {
        Ok(())
    }
    fn drop_database(&self, _: &str) -> DbResult<()> {
        Ok(())
    }
    fn drop_table(&self, _: &str, _: &str) -> DbResult<()> {
        Ok(())
    }
    fn truncate_table(&self, _: &str, _: &str) -> DbResult<()> {
        Ok(())
    }
    fn execute_transaction(&self, _: &[Statement]) -> DbResult<()> {
        Ok(())
    }
}

fn shared() -> Shared {
    Shared {
        settings: AppSettings::default(),
        profiles: Vec::new(),
        table_states: TableStates::default(),
    }
}

fn data(ids: &[&str]) -> QueryResult {
    QueryResult {
        columns: vec!["id".into()],
        rows: ids.iter().map(|id| vec![Some(id.to_string())]).collect(),
        ..Default::default()
    }
}

fn column(name: &str, key: &str, extra: &str) -> ColumnInfo {
    ColumnInfo {
        name: name.into(),
        col_type: "INT".into(),
        is_nullable: true,
        default_val: None,
        column_key: key.into(),
        extra: extra.into(),
    }
}

/// Sesión conectada con `db.t` (clave primaria `id`) abierta y sus datos
/// y estructura cargados.
fn session_with_table() -> Session {
    let mut s = Session::new(0);
    s.state = ConnState::Connected(Arc::new(FakeBackend));
    s.schema.selected_node = Some(SelectedNode::Table {
        db: "db".into(),
        table: "t".into(),
    });
    s.table_view.table_data = Some(data(&["1", "2"]));
    s.schema.columns_info.insert(
        ("db".into(), "t".into()),
        Loadable::Ready(vec![column("id", "PRI", ""), column("v", "", "")]),
    );
    s.sync_primary_keys();
    s
}

#[test]
fn recargar_con_cambios_pendientes_pide_confirmacion() {
    let mut s = session_with_table();
    s.table_view.pending_deletes.insert(0);
    s.request_reload(&mut shared(), Reload::Refresh);
    assert!(matches!(
        s.modal,
        ModalState::ConfirmReload {
            pending_count: 1,
            ..
        }
    ));
    assert!(
        matches!(s.state, ConnState::Connected(_)),
        "no debe recargar"
    );
    assert_eq!(s.table_view.pending_deletes.len(), 1);
}

#[test]
fn recargar_sin_cambios_lanza_la_query() {
    let mut s = session_with_table();
    s.request_reload(&mut shared(), Reload::Refresh);
    assert!(matches!(s.modal, ModalState::None));
    assert!(s.table_view.query_loading);
}

#[test]
fn descartar_y_recargar_limpia_los_cambios() {
    let mut s = session_with_table();
    s.table_view.pending_deletes.insert(1);
    s.handle_modal_action(
        ModalAction::DiscardAndReload(Reload::Refresh),
        &mut shared(),
    );
    assert_eq!(s.pending_count(), 0);
    assert!(s.table_view.query_loading);
}

#[test]
fn datos_nuevos_no_heredan_cambios_por_posicion() {
    let mut s = session_with_table();
    s.table_view.pending_deletes.insert(0);
    s.apply_job(
        &mut shared(),
        JobResult::TableData {
            generation: s.table_view.data_generation,
            sql: String::new(),
            elapsed_ms: 0,
            result: Ok(data(&["7", "8"])),
        },
    );
    assert_eq!(s.pending_count(), 0);
}

#[test]
fn los_datos_de_una_consulta_sustituida_se_descartan() {
    let mut s = session_with_table();
    s.request_reload(&mut shared(), Reload::Refresh);
    let stale = s.table_view.data_generation;
    s.request_reload(&mut shared(), Reload::Refresh);
    s.apply_job(
        &mut shared(),
        JobResult::TableData {
            generation: stale,
            sql: String::new(),
            elapsed_ms: 0,
            result: Ok(data(&["viejo"])),
        },
    );
    assert_eq!(s.table_view.table_data, Some(data(&["1", "2"])));
    assert!(
        s.table_view.query_loading,
        "la consulta nueva sigue en curso"
    );
}

fn open_table(s: &Session) -> Option<(&str, &str)> {
    match &s.schema.selected_node {
        Some(SelectedNode::Table { db, table }) => Some((db.as_str(), table.as_str())),
        _ => None,
    }
}

#[test]
fn crear_tabla_conserva_la_vista_y_los_cambios() {
    let mut s = session_with_table();
    s.active_tab = ActiveTab::Datos;
    s.table_view.pending_deletes.insert(0);
    let mut col = NewColumnDef::new();
    col.name = "id".into();
    let mut sh = shared();
    s.handle_modal_action(
        ModalAction::CreateTable {
            db: "db".into(),
            name: "nueva".into(),
            columns: vec![col],
        },
        &mut sh,
    );
    finish_jobs(&mut s, &mut sh);
    assert_eq!(open_table(&s), Some(("db", "t")));
    assert_eq!(s.active_tab, ActiveTab::Datos);
    assert_eq!(s.pending_count(), 1);
}

#[test]
fn borrar_la_tabla_abierta_la_cierra() {
    let mut s = session_with_table();
    s.active_tab = ActiveTab::Datos;
    s.table_view.pending_deletes.insert(0);
    let action = ModalAction::DropTable {
        db: "db".into(),
        table: "t".into(),
    };
    let mut sh = shared();
    s.handle_modal_action(action, &mut sh);
    finish_jobs(&mut s, &mut sh);
    assert_eq!(
        s.schema.selected_node,
        Some(SelectedNode::Database("db".into()))
    );
    assert_eq!(s.active_tab, ActiveTab::Bbdd);
    assert!(s.table_view.table_data.is_none());
    assert_eq!(s.pending_count(), 0);
}

#[test]
fn borrar_otra_tabla_no_toca_la_abierta() {
    let mut s = session_with_table();
    s.table_view.pending_deletes.insert(0);
    let action = ModalAction::DropTable {
        db: "db".into(),
        table: "otra".into(),
    };
    let mut sh = shared();
    s.handle_modal_action(action, &mut sh);
    finish_jobs(&mut s, &mut sh);
    assert_eq!(open_table(&s), Some(("db", "t")));
    assert_eq!(s.pending_count(), 1);
}

#[test]
fn borrar_la_bbdd_desde_una_pestana_sql_no_cambia_de_pestana() {
    let mut s = session_with_table();
    s.active_tab = ActiveTab::Sql(0);
    let mut sh = shared();
    s.handle_modal_action(ModalAction::DropDatabase("db".into()), &mut sh);
    finish_jobs(&mut s, &mut sh);
    assert_eq!(s.schema.selected_node, Some(SelectedNode::Server));
    assert_eq!(s.active_tab, ActiveTab::Sql(0));
}

#[test]
fn vaciar_la_tabla_abierta_la_recarga_sin_cambios() {
    let mut s = session_with_table();
    s.active_tab = ActiveTab::Datos;
    s.table_view.pending_edits.push(PendingEdit {
        row_idx: 0,
        col_idx: 0,
        new_value: "9".into(),
        is_null: false,
    });
    let action = ModalAction::TruncateTable {
        db: "db".into(),
        table: "t".into(),
    };
    let mut sh = shared();
    s.handle_modal_action(action, &mut sh);
    finish_jobs(&mut s, &mut sh);
    assert_eq!(open_table(&s), Some(("db", "t")));
    assert_eq!(s.pending_count(), 0);
    // Recargada: FakeBackend devuelve una tabla vacía
    assert_eq!(
        s.table_view.table_data.as_ref().map(|d| d.rows.len()),
        Some(0)
    );
}

fn edit(row_idx: usize, col_idx: usize, value: Option<&str>) -> PendingEdit {
    PendingEdit {
        row_idx,
        col_idx,
        new_value: value.unwrap_or_default().into(),
        is_null: value.is_none(),
    }
}

#[test]
fn cambios_pendientes_como_sentencias_con_parametros() {
    let mut s = session_with_table();
    s.table_view.table_data = Some(QueryResult {
        columns: vec!["id".into(), "v".into()],
        rows: vec![
            vec![Some("1".into()), Some("a".into())],
            vec![Some("2".into()), None],
        ],
        ..Default::default()
    });
    s.table_view.pending_deletes.insert(1);
    // Clave primaria y otra columna de la misma fila: un único UPDATE
    s.table_view.pending_edits = vec![edit(0, 0, Some("10")), edit(0, 1, None)];
    s.table_view.pending_inserts.push(PendingInsert {
        values: vec![
            InsertValue::Value("3".into()),
            InsertValue::Value("x'y".into()),
        ],
        auto_cols: vec![false, false],
    });

    let stmts = s.pending_statements().unwrap();
    let p = |v: &[Option<&str>]| v.iter().map(|x| x.map(String::from)).collect::<Vec<_>>();
    assert_eq!(
        stmts,
        [
            Statement {
                sql: "DELETE FROM `db`.`t` WHERE `id` = ?".into(),
                params: p(&[Some("2")]),
                single_row: true,
            },
            Statement {
                sql: "UPDATE `db`.`t` SET `id` = ?, `v` = ? WHERE `id` = ?".into(),
                params: p(&[Some("10"), None, Some("1")]),
                single_row: true,
            },
            Statement {
                sql: "INSERT INTO `db`.`t` (`id`, `v`) VALUES (?, ?)".into(),
                params: p(&[Some("3"), Some("x'y")]),
                single_row: false,
            },
        ]
    );
}

#[test]
fn sin_clave_primaria_no_se_editan_filas() {
    let mut s = session_with_table();
    s.table_view.primary_keys.clear();
    s.table_view.pending_deletes.insert(0);
    s.commit_pending_changes(&mut shared(), AfterCommit::ReloadData);
    assert!(matches!(s.modal, ModalState::Error { .. }));
    assert_eq!(s.pending_count(), 1);
}

#[test]
#[ignore = "necesita RUSTQL_TEST_MYSQL"]
fn guardar_cambios_contra_el_servidor() {
    let b = crate::db::test_server::backend();
    b.execute_write("DROP DATABASE IF EXISTS rustql_test_grid")
        .unwrap();
    b.execute_write("CREATE DATABASE rustql_test_grid").unwrap();
    b.execute_write("CREATE TABLE rustql_test_grid.t (id VARCHAR(20) PRIMARY KEY, v TEXT)")
        .unwrap();
    b.execute_write("INSERT INTO rustql_test_grid.t VALUES ('a','uno'),('b','dos'),('c','tres')")
        .unwrap();
    let select = "SELECT * FROM rustql_test_grid.t ORDER BY id";

    let mut s = Session::new(0);
    s.table_view.table_data = Some(b.execute_query(select).unwrap());
    let columns = b.get_columns_info("rustql_test_grid", "t").unwrap();
    s.schema.columns_info.insert(
        ("rustql_test_grid".into(), "t".into()),
        Loadable::Ready(columns),
    );
    s.state = ConnState::Connected(Arc::new(b));
    s.schema.selected_node = Some(SelectedNode::Table {
        db: "rustql_test_grid".into(),
        table: "t".into(),
    });
    s.sync_primary_keys();
    s.table_view.pending_deletes.insert(2);
    s.table_view.pending_edits = vec![edit(0, 0, Some("a'\\x")), edit(0, 1, None)];
    s.table_view.pending_inserts.push(PendingInsert {
        values: vec![
            InsertValue::Value("d".into()),
            InsertValue::Value("it's \\ fine".into()),
        ],
        auto_cols: vec![false, false],
    });

    let mut sh = shared();
    s.commit_pending_changes(&mut sh, AfterCommit::ReloadData);
    finish_jobs(&mut s, &mut sh);
    let saved = s.pending_count() == 0;
    let ConnState::Connected(b) = &s.state else {
        unreachable!()
    };
    let rows = b.execute_query(select).map(|r| r.rows);
    b.execute_write("DROP DATABASE rustql_test_grid").unwrap();

    assert!(saved, "{:?}", s.modal);
    let row = |id: &str, v: Option<&str>| vec![Some(id.to_string()), v.map(String::from)];
    assert_eq!(
        rows.unwrap(),
        [
            row("a'\\x", None),
            row("b", Some("dos")),
            row("d", Some("it's \\ fine")),
        ]
    );
}

#[test]
fn volver_al_valor_original_quita_el_cambio() {
    let mut s = session_with_table();
    let cell = RowRef::Existing(0);
    s.record_edit(cell, 0, Some("9".into()));
    assert_eq!(s.table_view.pending_edits.len(), 1);
    s.record_edit(cell, 0, Some("1".into()));
    assert!(s.table_view.pending_edits.is_empty());
}

#[test]
fn null_sobre_null_no_es_un_cambio() {
    let mut s = session_with_table();
    s.table_view.table_data.as_mut().unwrap().rows[0][0] = None;
    s.record_edit(RowRef::Existing(0), 0, None);
    assert!(s.table_view.pending_edits.is_empty());
    s.record_edit(RowRef::Existing(0), 0, Some(String::new()));
    assert_eq!(s.table_view.pending_edits.len(), 1, "NULL → '' sí cambia");
}

#[test]
#[ignore = "necesita RUSTQL_TEST_MYSQL"]
fn vaciar_todas_sin_haber_desplegado_el_arbol() {
    let b = crate::db::test_server::backend();
    for sql in [
        "DROP DATABASE IF EXISTS rustql_test_truncate",
        "CREATE DATABASE rustql_test_truncate",
        "CREATE TABLE rustql_test_truncate.padre (id INT PRIMARY KEY)",
        "CREATE TABLE rustql_test_truncate.hija (id INT, \
         FOREIGN KEY (id) REFERENCES rustql_test_truncate.padre (id))",
        "CREATE TABLE rustql_test_truncate.suelta (id INT)",
        "INSERT INTO rustql_test_truncate.padre VALUES (1)",
        "INSERT INTO rustql_test_truncate.hija VALUES (1)",
        "INSERT INTO rustql_test_truncate.suelta VALUES (1), (2)",
    ] {
        b.execute_write(sql).unwrap();
    }
    let mut s = Session::new(0);
    s.state = ConnState::Connected(Arc::new(b));
    assert!(s.schema.tables_cache.is_empty());

    let mut sh = shared();
    s.handle_modal_action(
        ModalAction::TruncateAllTables("rustql_test_truncate".into()),
        &mut sh,
    );
    finish_jobs(&mut s, &mut sh);
    let ConnState::Connected(b) = &s.state else {
        unreachable!()
    };
    let count = |t: &str| {
        b.execute_query(&format!("SELECT COUNT(*) FROM rustql_test_truncate.{t}"))
            .unwrap()
            .rows[0][0]
            .clone()
    };
    let (suelta, hija, padre) = (count("suelta"), count("hija"), count("padre"));
    b.execute_write("DROP DATABASE rustql_test_truncate")
        .unwrap();

    assert_eq!(suelta, Some("0".into()));
    assert_eq!(hija, Some("0".into()));
    // La referenciada por una clave foránea no se puede truncar: se avisa
    assert_eq!(padre, Some("1".into()));
    assert!(
        matches!(&s.modal, ModalState::Error { message, .. } if message.starts_with("padre:")),
        "{:?}",
        s.modal
    );
}

/// `db.t` con columnas id (AUTO_INCREMENT), v y n.
fn session_with_auto_id() -> Session {
    let mut s = session_with_table();
    s.table_view.table_data = Some(QueryResult {
        columns: vec!["id".into(), "v".into(), "n".into()],
        rows: vec![vec![Some("1".into()), Some("a".into()), None]],
        ..Default::default()
    });
    s.schema.columns_info.insert(
        ("db".into(), "t".into()),
        Loadable::Ready(vec![
            column("id", "PRI", "auto_increment"),
            column("v", "", ""),
            column("n", "", ""),
        ]),
    );
    s
}

#[test]
fn fila_nueva_deja_los_valores_por_defecto_al_servidor() {
    let mut s = session_with_auto_id();
    let row = s.new_insert_row(None).unwrap();
    // Antes: '' en v y n, y el texto «CURRENT_TIMESTAMP» como valor
    assert_eq!(row.values, vec![InsertValue::Default; 3]);
    s.table_view.pending_inserts.push(row);
    s.record_edit(RowRef::Insert(0), 1, Some("x".into()));
    let stmts = s.pending_statements().unwrap();
    assert_eq!(stmts[0].sql, "INSERT INTO `db`.`t` (`v`) VALUES (?)");
    assert_eq!(stmts[0].params, [Some("x".to_string())]);
}

#[test]
fn fila_nueva_sin_tocar_inserta_todo_por_defecto() {
    let mut s = session_with_auto_id();
    let row = s.new_insert_row(None).unwrap();
    s.table_view.pending_inserts.push(row);
    let stmts = s.pending_statements().unwrap();
    assert_eq!(stmts[0].sql, "INSERT INTO `db`.`t` () VALUES ()");
}

#[test]
fn clonar_fila_copia_valores_y_nulls_salvo_el_auto_increment() {
    let s = session_with_auto_id();
    let row = s.new_insert_row(Some(0)).unwrap();
    assert_eq!(
        row.values,
        [
            InsertValue::Default,
            InsertValue::Value("a".into()),
            InsertValue::Null
        ]
    );
}

#[test]
#[ignore = "necesita RUSTQL_TEST_MYSQL"]
fn fila_nueva_contra_el_servidor() {
    let b = crate::db::test_server::backend();
    b.execute_write("DROP DATABASE IF EXISTS rustql_test_insert")
        .unwrap();
    b.execute_write("CREATE DATABASE rustql_test_insert")
        .unwrap();
    b.execute_write(
        "CREATE TABLE rustql_test_insert.t (id INT AUTO_INCREMENT PRIMARY KEY, \
         creado DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP, n INT NULL, \
         v VARCHAR(10) NOT NULL DEFAULT 'x')",
    )
    .unwrap();
    let select = "SELECT id, creado IS NOT NULL, n, v FROM rustql_test_insert.t";

    let mut s = Session::new(0);
    s.table_view.table_data = Some(QueryResult {
        columns: vec!["id".into(), "creado".into(), "n".into(), "v".into()],
        rows: Vec::new(),
        ..Default::default()
    });
    let key = ("rustql_test_insert".to_string(), "t".to_string());
    let columns = b.get_columns_info(&key.0, &key.1).unwrap();
    s.schema
        .columns_info
        .insert(key.clone(), Loadable::Ready(columns));
    s.schema.selected_node = Some(SelectedNode::Table {
        db: key.0,
        table: key.1,
    });
    s.state = ConnState::Connected(Arc::new(b));
    let row = s.new_insert_row(None).unwrap();
    s.table_view.pending_inserts.push(row);

    let mut sh = shared();
    s.commit_pending_changes(&mut sh, AfterCommit::ReloadData);
    finish_jobs(&mut s, &mut sh);
    let saved = s.pending_count() == 0;
    let ConnState::Connected(b) = &s.state else {
        unreachable!()
    };
    let rows = b.execute_query(select).map(|r| r.rows);
    b.execute_write("DROP DATABASE rustql_test_insert").unwrap();

    assert!(saved, "{:?}", s.modal);
    let some = |v: &str| Some(v.to_string());
    assert_eq!(rows.unwrap(), [[some("1"), some("1"), None, some("x")]]);
}

/// Espera a que terminen los trabajos en segundo plano y los aplica.
fn finish_jobs(s: &mut Session, sh: &mut Shared) {
    let start = Instant::now();
    while s.jobs.pending() > 0 {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "trabajos sin terminar"
        );
        s.poll_jobs(&egui::Context::default(), sh);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn tables<'a>(s: &'a Session, db: &str) -> Option<&'a Vec<String>> {
    s.schema.tables_cache.get(db).and_then(Loadable::ready)
}

fn ticket<T>(slot: Option<&Loadable<T>>) -> u64 {
    match slot {
        Some(Loadable::Loading(ticket)) => *ticket,
        _ => panic!("no está cargando"),
    }
}

#[test]
fn un_fallo_de_esquema_no_se_reintenta_solo() {
    let mut s = session_with_table();
    s.schema.databases = Loadable::Failed("sin permiso".into());
    s.request_tab_data();
    assert!(matches!(s.schema.databases, Loadable::Failed(_)));
    // Refrescar sí lo vuelve a pedir
    s.schema.clear();
    s.request_tab_data();
    assert!(matches!(s.schema.databases, Loadable::Loading(_)));
}

#[test]
fn la_respuesta_de_una_peticion_invalidada_se_descarta() {
    let mut s = session_with_table();
    s.request_tables("db");
    let old = ticket(s.schema.tables_cache.get("db"));
    s.invalidate_schema("db");
    s.request_tables("db");
    let new = ticket(s.schema.tables_cache.get("db"));
    s.apply_schema_job(SchemaResult::Tables {
        ticket: old,
        db: "db".into(),
        result: Ok(vec!["vieja".into()]),
    });
    assert!(matches!(s.schema.tables_cache["db"], Loadable::Loading(t) if t == new));
    s.apply_schema_job(SchemaResult::Tables {
        ticket: new,
        db: "db".into(),
        result: Ok(vec!["nueva".into()]),
    });
    assert_eq!(tables(&s, "db"), Some(&vec!["nueva".to_string()]));
}

#[test]
fn el_buscador_pide_todas_las_tablas_en_una_consulta() {
    let mut s = session_with_table();
    s.schema.databases = Loadable::Ready(vec!["a".into(), "b".into(), "c".into()]);
    s.schema
        .tables_cache
        .insert("a".into(), Loadable::Ready(vec!["ya".into()]));
    s.request_all_tables();
    let t = ticket(s.schema.tables_cache.get("b"));
    assert_eq!(
        ticket(s.schema.tables_cache.get("c")),
        t,
        "una sola petición"
    );
    s.apply_schema_job(SchemaResult::AllTables {
        ticket: t,
        result: Ok(vec![("b".into(), "x".into()), ("otra".into(), "y".into())]),
    });
    assert_eq!(tables(&s, "a"), Some(&vec!["ya".to_string()]));
    assert_eq!(tables(&s, "b"), Some(&vec!["x".to_string()]));
    assert_eq!(tables(&s, "c"), Some(&Vec::new()), "BBDD sin tablas");
}

#[test]
fn exportar_rellena_las_tablas_cuando_llegan() {
    let mut s = session_with_table();
    s.open_export(&shared(), "db".into(), Some("t2".into()));
    let t = ticket(s.schema.tables_cache.get("db"));
    s.apply_schema_job(SchemaResult::Tables {
        ticket: t,
        db: "db".into(),
        result: Ok(vec!["t1".into(), "t2".into()]),
    });
    let ModalState::ExportData { tables, .. } = &s.modal else {
        panic!("{:?}", s.modal)
    };
    let marked: Vec<_> = tables
        .iter()
        .map(|e| (e.name.as_str(), e.selected))
        .collect();
    assert_eq!(marked, [("t1", false), ("t2", true)]);
}

#[test]
fn la_clave_primaria_sale_de_la_estructura() {
    let mut s = session_with_table();
    s.reload_now(&mut shared(), Reload::Refresh);
    let t = ticket(s.schema.columns_info.get(&("db".into(), "t".into())));
    s.apply_schema_job(SchemaResult::Structure {
        ticket: t,
        db: "db".into(),
        table: "t".into(),
        result: Ok(vec![column("a", "PRI", ""), column("b", "PRI", "")]),
    });
    assert_eq!(s.table_view.primary_keys, ["a", "b"]);
}

#[test]
fn el_formulario_espera_al_resultado_de_la_escritura() {
    let mut s = session_with_table();
    s.modal = ModalState::CreateDatabase {
        name: "nueva".into(),
        charset: "utf8mb4".into(),
        error: None,
    };
    let mut sh = shared();
    let action = ModalAction::CreateDatabase {
        name: "nueva".into(),
        charset: "utf8mb4".into(),
    };
    s.handle_modal_action(action, &mut sh);
    assert!(matches!(s.modal, ModalState::CreateDatabase { .. }));
    assert!(s.is_busy());
    finish_jobs(&mut s, &mut sh);
    assert!(matches!(s.modal, ModalState::None));
    assert!(!s.is_busy());
}

#[test]
fn mientras_se_guarda_no_se_recarga_ni_se_cambia_de_tabla() {
    let mut s = session_with_table();
    s.table_view.pending_deletes.insert(0);
    let mut sh = shared();
    s.commit_pending_changes(&mut sh, AfterCommit::ReloadData);
    assert!(s.table_view.saving);
    s.request_reload(&mut sh, Reload::Refresh);
    assert!(
        matches!(s.modal, ModalState::None),
        "sin diálogo de recarga"
    );
    s.switch_to_table(&mut sh, "db".into(), "otra".into(), ActiveTab::Datos);
    assert_eq!(open_table(&s), Some(("db", "t")));
    finish_jobs(&mut s, &mut sh);
    assert!(!s.table_view.saving);
    assert_eq!(s.pending_count(), 0);
}

#[test]
fn el_resultado_del_editor_va_a_su_pestana() {
    let mut s = session_with_table();
    s.sql_tabs = vec![SqlTab::default(), SqlTab::default()];
    s.run_editor(1, "SELECT 1".into(), String::new(), 10);
    assert!(s.sql_tabs[1].running.is_some());
    // Se cierra la primera: la que ejecuta pasa a ser la 0
    s.sql_tabs.remove(0);
    finish_jobs(&mut s, &mut shared());
    assert!(s.sql_tabs[0].running.is_none());
    assert!(matches!(s.sql_tabs[0].result, Some(Ok(_))));
    assert_eq!(s.query_log.len(), 1);
}

#[test]
fn cancelar_termina_la_consulta_del_editor() {
    let mut s = session_with_table();
    let mut sh = shared();
    s.active_tab = ActiveTab::Sql(0);
    s.run_editor(0, "LENTA".into(), String::new(), 10);
    assert!(s.sql_tabs[0].running.is_some());

    let signals = || FrameSignals {
        sql: SqlEditorAction::Cancel,
        ..Default::default()
    };
    let ctx = egui::Context::default();
    s.apply_signals(&ctx, &mut sh, signals(), FrameKeys::default());
    // Pedirlo dos veces no lanza dos cancelaciones
    let pending = s.jobs.pending();
    s.apply_signals(&ctx, &mut sh, signals(), FrameKeys::default());
    assert_eq!(s.jobs.pending(), pending);

    finish_jobs(&mut s, &mut sh);
    let tab = &s.sql_tabs[0];
    assert!(tab.running.is_none() && tab.cancel.is_none());
    assert_eq!(tab.result, Some(Err(QUERY_CANCELLED.to_string())));
}

#[test]
fn cerrar_una_pestana_sql_cancela_su_consulta() {
    let mut s = session_with_table();
    let mut sh = shared();
    s.sql_tabs = vec![SqlTab::default(), SqlTab::default()];
    s.run_editor(1, "LENTA".into(), String::new(), 10);
    s.close_sql_tab(&mut sh, 1);
    assert_eq!(s.sql_tabs.len(), 1);
    // Si no se cancelara, el hilo no terminaría nunca (ni el test)
    finish_jobs(&mut s, &mut sh);
}

#[test]
fn un_puerto_invalido_no_conecta_al_anterior() {
    let mut s = Session::new(0);
    let sh = shared();
    s.params.host = "db.example".into();
    s.port_str = "33o6".into();
    s.start_connect(&sh);
    assert!(matches!(s.state, ConnState::Disconnected));
    let err = s.error_message.as_deref().unwrap_or_default();
    assert!(err.contains("33o6"), "{err}");

    s.params.host.clear();
    s.port_str = "3306".into();
    s.start_connect(&sh);
    assert!(matches!(s.state, ConnState::Disconnected), "sin host");
}

#[test]
fn guardar_un_favorito_sin_nombre_avisa() {
    let mut s = Session::new(0);
    let mut sh = shared();
    let ctx = egui::Context::default();
    s.handle_profile_form(&ctx, &mut sh, None, Some(SaveProfile::Update), false);
    assert!(sh.profiles.is_empty());
    assert!(s.error_message.is_some());
}

#[test]
fn guardar_como_nuevo_no_toca_el_favorito_seleccionado() {
    let mut s = Session::new(0);
    let mut sh = shared();
    let ctx = egui::Context::default();
    s.params.host = "uno.example".into();
    s.profile_name = "Uno".into();
    s.handle_profile_form(&ctx, &mut sh, None, Some(SaveProfile::Update), false);
    assert_eq!(s.selected_profile_idx, Some(0));

    // Mismo nombre: sería un duplicado indistinguible
    s.params.host = "dos.example".into();
    s.handle_profile_form(&ctx, &mut sh, None, Some(SaveProfile::AsNew), false);
    assert_eq!(sh.profiles.len(), 1);
    assert!(s.error_message.is_some());

    s.profile_name = "Dos".into();
    s.handle_profile_form(&ctx, &mut sh, None, Some(SaveProfile::AsNew), false);
    let hosts: Vec<_> = sh.profiles.iter().map(|p| p.host.as_str()).collect();
    assert_eq!(hosts, ["uno.example", "dos.example"]);
    assert_ne!(sh.profiles[0].id, sh.profiles[1].id);
    assert_eq!(s.selected_profile_idx, Some(1));
    assert!(s.error_message.is_none());
}

#[test]
fn cerrar_una_pestana_sql_con_texto_pide_confirmacion() {
    let mut s = session_with_table();
    let mut sh = shared();
    s.sql_tabs = vec![SqlTab::default(), SqlTab::default()];
    s.sql_tabs[1].sql_text = "SELECT importante".into();
    s.active_tab = ActiveTab::Sql(1);
    let close = |idx| FrameSignals {
        tab_bar: crate::ui::tab_bar::TabBarResult {
            close_sql_tab: Some(idx),
            ..Default::default()
        },
        ..Default::default()
    };
    let ctx = egui::Context::default();

    s.apply_signals(&ctx, &mut sh, close(1), FrameKeys::default());
    assert_eq!(s.sql_tabs.len(), 2, "con texto no se cierra sin más");
    assert!(matches!(
        s.modal,
        ModalState::ConfirmCloseSqlTab { idx: 1, .. }
    ));
    s.handle_modal_action(ModalAction::CloseSqlTab(1), &mut sh);
    assert_eq!(s.sql_tabs.len(), 1);
    assert_eq!(s.active_tab, ActiveTab::Sql(0));

    // Vacía: se cierra directamente (y queda una nueva en su lugar)
    s.apply_signals(&ctx, &mut sh, close(0), FrameKeys::default());
    assert!(matches!(s.modal, ModalState::None));
    assert_eq!(s.sql_tabs.len(), 1);
}

#[test]
fn limpiar_el_editor_pregunta_y_luego_guarda_la_pestana_vacia() {
    let mut s = session_with_table();
    let mut sh = shared();
    s.active_tab = ActiveTab::Sql(0);
    s.sql_tabs[0].sql_text = "SELECT importante".into();
    let signals = FrameSignals {
        sql: SqlEditorAction::Clear,
        ..Default::default()
    };
    s.apply_signals(
        &egui::Context::default(),
        &mut sh,
        signals,
        FrameKeys::default(),
    );
    assert_eq!(s.sql_tabs[0].sql_text, "SELECT importante");
    assert!(matches!(
        s.modal,
        ModalState::ConfirmClearSql { idx: 0, .. }
    ));
    s.handle_modal_action(ModalAction::ClearSql(0), &mut sh);
    assert!(s.sql_tabs[0].sql_text.is_empty());
    assert_eq!(saved_sql(&s, &sh), [""]);
}

#[test]
fn renombrar_una_pestana_sql_se_guarda_y_vacio_vuelve_al_numero() {
    let mut s = session_with_table();
    let mut sh = shared();
    let ctx = egui::Context::default();
    let rename = |name: &str| FrameSignals {
        tab_bar: crate::ui::tab_bar::TabBarResult {
            renamed: Some((0, name.to_string())),
            ..Default::default()
        },
        ..Default::default()
    };
    s.apply_signals(&ctx, &mut sh, rename("Informe"), FrameKeys::default());
    assert_eq!(s.sql_tabs[0].title(0), "Informe");
    let saved = &sh.settings.sql_tabs_by_conn[&s.params.key()];
    assert_eq!(saved[0].name.as_deref(), Some("Informe"));
    // Al reconectar se restaura con su nombre
    assert_eq!(SqlTab::restore(&saved[0]).title(3), "Informe");

    s.apply_signals(&ctx, &mut sh, rename(""), FrameKeys::default());
    assert_eq!(s.sql_tabs[0].title(0), "SQL 1");
}

#[test]
fn una_consulta_del_log_se_abre_en_una_pestana_sql_nueva() {
    let mut s = session_with_table();
    let mut sh = shared();
    s.open_sql_tab(&mut sh, None, "SELECT del log".into());
    assert_eq!(s.active_tab, ActiveTab::Sql(1));
    assert_eq!(s.sql_tabs[1].sql_text, "SELECT del log");
    // Con la BBDD de la tabla abierta como contexto, y ya guardada
    assert_eq!(s.sql_tabs[1].db_context, "db");
    assert_eq!(saved_sql(&s, &sh), ["", "SELECT del log"]);
}

#[test]
fn ctrl_w_cierra_la_pestana_sql_activa_y_solo_esa() {
    let mut s = session_with_table();
    let mut sh = shared();
    let ctx = egui::Context::default();
    let ctrl_w = FrameKeys {
        ctrl_w: true,
        ..Default::default()
    };
    s.sql_tabs = vec![SqlTab::default(), SqlTab::default()];
    s.sql_tabs[1].sql_text = "SELECT importante".into();

    // En una pestaña que no es SQL no cierra nada
    s.active_tab = ActiveTab::Datos;
    s.apply_signals(&ctx, &mut sh, Default::default(), ctrl_w);
    assert_eq!(s.sql_tabs.len(), 2);
    assert!(matches!(s.modal, ModalState::None));

    // Con texto pregunta, igual que la × de la pestaña
    s.active_tab = ActiveTab::Sql(1);
    s.apply_signals(&ctx, &mut sh, Default::default(), ctrl_w);
    assert!(matches!(
        s.modal,
        ModalState::ConfirmCloseSqlTab { idx: 1, .. }
    ));
    s.modal = ModalState::None;

    s.active_tab = ActiveTab::Sql(0);
    s.apply_signals(&ctx, &mut sh, Default::default(), ctrl_w);
    assert_eq!(s.sql_tabs.len(), 1);
    assert_eq!(s.sql_tabs[0].sql_text, "SELECT importante");
}

#[test]
fn los_atajos_de_pestana_no_llegan_a_los_campos_de_texto() {
    let ctx = egui::Context::default();
    let key = |key, modifiers| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    };
    // Como llega Ctrl en Linux y Windows: es también la tecla de comando
    let ctrl = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
    let ctrl_shift = ctrl | egui::Modifiers::SHIFT;
    let frame = |events: Vec<egui::Event>| {
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut seen = None;
        ctx.run_ui(input, |ui| {
            let keys = FrameKeys::read(ui.ctx());
            // Lo que le queda por leer a un TextEdit dibujado después
            let left = ui.input(|i| i.events.len());
            seen = Some((keys.cycle_tab, keys.ctrl_w, left));
        })
        .drop_without_applying_deltas();
        seen.unwrap()
    };

    let tab = egui::Key::Tab;
    assert_eq!(frame(vec![key(tab, ctrl)]), (Some(true), false, 0));
    assert_eq!(frame(vec![key(tab, ctrl_shift)]), (Some(false), false, 0));
    assert_eq!(frame(vec![key(egui::Key::W, ctrl)]), (None, true, 0));
    // Tab a secas sigue siendo del widget con el foco
    assert_eq!(
        frame(vec![key(tab, egui::Modifiers::NONE)]),
        (None, false, 1)
    );
}

#[test]
fn ctrl_tab_recorre_las_pestanas_activas_y_da_la_vuelta() {
    let mut s = session_with_table();
    let mut sh = shared();
    let ctx = egui::Context::default();
    let cycle = |forward| FrameKeys {
        cycle_tab: Some(forward),
        ..Default::default()
    };
    s.sql_tabs = vec![SqlTab::default(), SqlTab::default()];
    s.active_tab = ActiveTab::Datos;

    let mut seen = Vec::new();
    for _ in 0..3 {
        s.apply_signals(&ctx, &mut sh, Default::default(), cycle(true));
        seen.push(s.active_tab);
    }
    assert_eq!(
        seen,
        [ActiveTab::Sql(0), ActiveTab::Sql(1), ActiveTab::Servidor]
    );
    s.apply_signals(&ctx, &mut sh, Default::default(), cycle(false));
    assert_eq!(s.active_tab, ActiveTab::Sql(1));

    // Sin tabla abierta, Estructura y Datos están desactivadas: se saltan
    s.schema.selected_node = Some(SelectedNode::Database("db".into()));
    s.active_tab = ActiveTab::Bbdd;
    s.apply_signals(&ctx, &mut sh, Default::default(), cycle(true));
    assert_eq!(s.active_tab, ActiveTab::Sql(0));
}

fn saved_sql(s: &Session, sh: &Shared) -> Vec<String> {
    sh.settings
        .sql_tabs_by_conn
        .get(&s.params.key())
        .map(|tabs| tabs.iter().map(|t| t.sql_text.clone()).collect())
        .unwrap_or_default()
}

#[test]
fn el_texto_sql_sin_ejecutar_se_guarda_al_salir_de_la_pestana() {
    let mut s = session_with_table();
    let mut sh = shared();
    s.active_tab = ActiveTab::Sql(0);
    s.sql_tabs[0].sql_text = "SELECT a medias".into();
    let signals = FrameSignals {
        tab_bar: crate::ui::tab_bar::TabBarResult {
            tab_clicked: Some(ActiveTab::Datos),
            ..Default::default()
        },
        ..Default::default()
    };
    s.apply_signals(
        &egui::Context::default(),
        &mut sh,
        signals,
        FrameKeys::default(),
    );
    assert_eq!(saved_sql(&s, &sh), ["SELECT a medias"]);
}

#[test]
fn el_texto_sql_se_guarda_al_desconectar_y_no_en_el_formulario() {
    let mut s = session_with_table();
    let mut sh = shared();
    s.sql_tabs[0].sql_text = "SELECT 1".into();
    let ctx = egui::Context::default();
    let signals = FrameSignals {
        sidebar: crate::ui::sidebar::SidebarOutput {
            action: crate::ui::sidebar::SidebarAction::Disconnect,
            ..Default::default()
        },
        ..Default::default()
    };
    s.apply_signals(&ctx, &mut sh, signals, FrameKeys::default());
    assert!(!s.is_connected());
    assert_eq!(saved_sql(&s, &sh), ["SELECT 1"]);

    // Ya en el formulario, no se guarda bajo unos parámetros cualesquiera
    s.sql_tabs[0].sql_text = "otra cosa".into();
    s.save_tabs_state(&mut sh);
    assert_eq!(saved_sql(&s, &sh), ["SELECT 1"]);
}

#[test]
fn desconectar_no_deja_el_editor_esperando() {
    let mut s = session_with_table();
    s.run_editor(0, "SELECT 1".into(), String::new(), 10);
    s.disconnect();
    assert!(s.sql_tabs[0].running.is_none());
}

#[test]
fn al_cambiar_de_tabla_no_se_ven_los_datos_de_la_anterior() {
    let mut s = session_with_table();
    let mut sh = shared();
    s.request_reload(&mut sh, Reload::Refresh);
    let old_query = s.table_view.data_generation;
    s.switch_to_table(&mut sh, "db".into(), "otra".into(), ActiveTab::Datos);
    assert!(s.table_view.table_data.is_none());
    s.apply_job(
        &mut sh,
        JobResult::TableData {
            generation: old_query,
            sql: String::new(),
            elapsed_ms: 0,
            result: Ok(data(&["de la anterior"])),
        },
    );
    assert!(s.table_view.table_data.is_none());
    assert!(s.table_view.query_loading);
}

/// Un frame en el que solo se pulsa F5.
fn press_f5(s: &mut Session, sh: &mut Shared) {
    let ctx = egui::Context::default();
    let f5 = FrameKeys {
        f5: true,
        ..Default::default()
    };
    s.apply_signals(&ctx, sh, Default::default(), f5);
}

#[test]
fn f5_en_una_pestana_sql_ejecuta_su_consulta() {
    let mut s = session_with_table();
    s.sql_tabs[0].sql_text = "SELECT 1".into();
    s.active_tab = ActiveTab::Sql(0);
    let mut sh = shared();
    press_f5(&mut s, &mut sh);
    assert!(s.sql_tabs[0].running.is_some());
    assert!(!s.table_view.query_loading, "no recarga la tabla abierta");
    finish_jobs(&mut s, &mut sh);
    assert!(matches!(s.sql_tabs[0].result, Some(Ok(_))));
}

#[test]
fn f5_en_estructura_relee_las_columnas_y_no_los_datos() {
    let mut s = session_with_table();
    s.active_tab = ActiveTab::Tabla;
    press_f5(&mut s, &mut shared());
    assert!(s.schema.open_table_columns().is_none());
    assert!(!s.table_view.query_loading);
    s.request_tab_data();
    let key = ("db".to_string(), "t".to_string());
    assert!(matches!(
        s.schema.columns_info.get(&key),
        Some(Loadable::Loading(_))
    ));
}

#[test]
fn f5_en_datos_recarga_la_tabla() {
    let mut s = session_with_table();
    s.active_tab = ActiveTab::Datos;
    press_f5(&mut s, &mut shared());
    assert!(s.table_view.query_loading);
}
