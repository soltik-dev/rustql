//! Contraseñas de los perfiles en el almacén del sistema: Secret Service en
//! Linux (gnome-keyring, KWallet…), Administrador de credenciales en Windows y
//! Llavero en macOS. Cada perfil guarda la suya bajo su `id`.
//!
//! Todas las funciones pueden bloquear —el llavero puede pedir que se
//! desbloquee y D-Bus no tiene timeout—, así que solo se llaman desde hilos de
//! trabajo, nunca desde el de la UI.
//!
//! Flatpak: el manifiesto necesita `--talk-name=org.freedesktop.secrets`.

use std::sync::OnceLock;

use keyring_core::{Entry, Error};

const SERVICE: &str = "rustql";

/// Resultado de inicializar el almacén (en Linux, conectar con D-Bus).
static STORE: OnceLock<Result<(), String>> = OnceLock::new();

/// Inicializa el almacén si aún no lo está y dice si está disponible.
pub fn init() -> bool {
    STORE
        .get_or_init(|| {
            let result = open_store();
            if let Err(e) = &result {
                log::warn!("Llavero del sistema no disponible: {e}");
            }
            result
        })
        .is_ok()
}

fn open_store() -> Result<(), String> {
    #[cfg(all(unix, not(target_os = "macos")))]
    let store = zbus_secret_service_keyring_store::Store::new();
    #[cfg(target_os = "windows")]
    let store = windows_native_keyring_store::Store::new();
    #[cfg(target_os = "macos")]
    let store = apple_native_keyring_store::keychain::Store::new();
    keyring_core::set_default_store(store.map_err(|e| e.to_string())?);
    Ok(())
}

/// Disponibilidad sin bloquear: `None` mientras no se ha comprobado.
pub fn available() -> Option<bool> {
    STORE.get().map(Result::is_ok)
}

fn entry(id: &str, label: &str) -> Result<Entry, String> {
    if !init() {
        return Err("no hay llavero del sistema disponible".into());
    }
    // Solo Secret Service admite etiqueta (la que se ve en Seahorse/KWallet).
    #[cfg(all(unix, not(target_os = "macos")))]
    let entry = Entry::new_with_modifiers(
        SERVICE,
        id,
        &std::collections::HashMap::from([("label", label)]),
    );
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    let entry = {
        let _ = label;
        Entry::new(SERVICE, id)
    };
    entry.map_err(describe)
}

/// Contraseña guardada bajo `id`, o `None` si no hay ninguna.
pub fn get(id: &str) -> Result<Option<String>, String> {
    match entry(id, SERVICE)?.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(Error::NoEntry) => Ok(None),
        Err(e) => Err(describe(e)),
    }
}

/// Guarda (o sustituye) la contraseña de `id`. `label` solo se usa al crearla.
pub fn set(id: &str, label: &str, password: &str) -> Result<(), String> {
    entry(id, label)?.set_password(password).map_err(describe)
}

/// Borra la contraseña de `id`; no es un error que no exista.
pub fn delete(id: &str) -> Result<(), String> {
    match entry(id, SERVICE)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(e) => Err(describe(e)),
    }
}

/// `delete` en un hilo propio, para la UI: el fallo solo se registra.
pub fn delete_in_background(id: String) {
    std::thread::spawn(move || {
        if let Err(e) = delete(&id) {
            log::warn!("No se pudo borrar la contraseña del llavero: {e}");
        }
    });
}

fn describe(e: Error) -> String {
    match e {
        Error::NoStorageAccess(err) => {
            format!("el llavero está bloqueado o se canceló el desbloqueo ({err})")
        }
        e => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Usa el llavero real del sistema: `cargo test -- --ignored secrets`.
    #[test]
    #[ignore]
    fn ida_y_vuelta_en_el_llavero() {
        let id = "rustql-test-ida-y-vuelta";
        set(id, "RustQL — prueba", "s3cr3t·ñ").unwrap();
        assert_eq!(get(id).unwrap().as_deref(), Some("s3cr3t·ñ"));
        set(id, "RustQL — prueba", "otra").unwrap();
        assert_eq!(get(id).unwrap().as_deref(), Some("otra"));
        delete(id).unwrap();
        assert_eq!(get(id).unwrap(), None);
        delete(id).unwrap();
    }
}
