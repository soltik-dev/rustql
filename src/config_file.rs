//! Ficheros JSON de configuración en ~/.config/rustql (ajustes, perfiles y
//! estado de las tablas).
//!
//! Se escriben en un temporal que luego se renombra encima del fichero: un
//! corte a mitad de escritura no lo deja truncado. Si uno no se puede
//! interpretar, se aparta como copia de seguridad antes de usar los valores
//! por defecto; si no, el siguiente guardado lo sobrescribiría y se perdería
//! (por ejemplo, todos los perfiles).

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Directorio de configuración. En los tests, uno temporal: ningún test
/// debe tocar la configuración real.
fn dir() -> Option<PathBuf> {
    if cfg!(test) {
        Some(std::env::temp_dir().join("rustql-tests"))
    } else {
        dirs::config_dir().map(|d| d.join("rustql"))
    }
}

/// Lee `name`. Si no existe o no se puede leer, devuelve el valor por
/// defecto; si no es un JSON válido, además lo aparta como `.bak`.
pub fn load<T: DeserializeOwned + Default>(name: &str) -> T {
    let Some(path) = dir().map(|d| d.join(name)) else {
        return T::default();
    };
    let data = match std::fs::read_to_string(&path) {
        Ok(data) => data,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return T::default(),
        Err(e) => {
            log::warn!("No se pudo leer {}: {e}", path.display());
            return T::default();
        }
    };
    match serde_json::from_str(&data) {
        Ok(value) => value,
        Err(e) => {
            let backup = backup_path(&path);
            match std::fs::rename(&path, &backup) {
                Ok(()) => log::warn!(
                    "{} no es válido ({e}): se ha apartado como {}",
                    path.display(),
                    backup.display()
                ),
                Err(re) => log::warn!(
                    "{} no es válido ({e}) y no se pudo apartar: {re}",
                    path.display()
                ),
            }
            T::default()
        }
    }
}

/// Guarda `value` en `name`. Con `private`, el fichero solo es legible por
/// el usuario (0600) y el directorio, 0700 (en Unix).
pub fn save<T: Serialize>(name: &str, value: &T, private: bool) {
    if let Err(e) = try_save(name, value, private) {
        log::warn!("No se pudo guardar {name}: {e}");
    }
}

fn try_save<T: Serialize>(name: &str, value: &T, private: bool) -> io::Result<()> {
    // File modes only exist on Unix; on Windows the user's profile folder is
    // already private to them.
    #[cfg(not(unix))]
    let _ = private;
    let dir = dir().ok_or_else(|| io::Error::other("sin directorio de configuración"))?;
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let data = serde_json::to_string_pretty(value)?;

    // Nombre único: dos guardados a la vez no comparten temporal.
    static NEXT_TMP: AtomicUsize = AtomicUsize::new(0);
    let tmp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT_TMP.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    }
    let written = options.open(&tmp).and_then(|mut file| {
        file.write_all(data.as_bytes())?;
        file.sync_all()
    });
    let result = written.and_then(|()| std::fs::rename(&tmp, dir.join(name)));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// `connections.json` → `connections.json.20261026-134500.bak`
fn backup_path(path: &Path) -> PathBuf {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{stamp}.bak"));
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fichero de prueba con nombre propio, que se borra al terminar.
    struct TestFile(&'static str);

    impl TestFile {
        fn path(&self) -> PathBuf {
            dir().unwrap().join(self.0)
        }

        fn backups(&self) -> Vec<PathBuf> {
            let prefix = format!("{}.", self.0);
            // El directorio no existe hasta el primer guardado
            let Ok(entries) = std::fs::read_dir(dir().unwrap()) else {
                return Vec::new();
            };
            entries
                .filter_map(|e| Some(e.ok()?.path()))
                .filter(|p| {
                    let name = p.file_name().unwrap().to_string_lossy().into_owned();
                    name.starts_with(&prefix) && name.ends_with(".bak")
                })
                .collect()
        }
    }

    impl Drop for TestFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(self.path());
            for backup in self.backups() {
                let _ = std::fs::remove_file(backup);
            }
        }
    }

    #[test]
    fn ida_y_vuelta() {
        let file = TestFile("ida_y_vuelta.json");
        save(file.0, &vec!["a".to_string(), "b".to_string()], false);
        let back: Vec<String> = load(file.0);
        assert_eq!(back, ["a", "b"]);
    }

    #[test]
    fn sin_fichero_da_el_valor_por_defecto() {
        let file = TestFile("no_existe.json");
        let back: Vec<String> = load(file.0);
        assert!(back.is_empty());
        assert!(file.backups().is_empty());
    }

    #[test]
    fn un_json_invalido_se_aparta_en_vez_de_perderse() {
        let file = TestFile("corrupto.json");
        std::fs::create_dir_all(dir().unwrap()).unwrap();
        std::fs::write(file.path(), "[{\"name\": \"Prod\", ").unwrap();

        let back: Vec<String> = load(file.0);
        assert!(back.is_empty());
        assert!(!file.path().exists());
        let backups = file.backups();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&backups[0]).unwrap(),
            "[{\"name\": \"Prod\", "
        );
    }

    #[cfg(unix)]
    #[test]
    fn privado_solo_para_el_usuario() {
        use std::os::unix::fs::PermissionsExt;
        let file = TestFile("privado.json");
        save(file.0, &1, true);
        let mode = std::fs::metadata(file.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
