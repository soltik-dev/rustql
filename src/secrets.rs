//! Profile passwords in the system store: Secret Service on Linux
//! (gnome-keyring, KWallet…), Credential Manager on Windows and Keychain
//! on macOS. Each profile keeps its own under its `id`.
//!
//! Every function may block (the keyring may ask to be unlocked and D-Bus
//! has no timeout), so they are only called from worker threads, never
//! from the UI thread.
//!
//! Flatpak: the manifest needs `--talk-name=org.freedesktop.secrets`.

use std::sync::OnceLock;

use keyring_core::{Entry, Error};

use crate::i18n::t;

const SERVICE: &str = "rustql";

/// Result of initializing the store (on Linux, connecting to D-Bus).
static STORE: OnceLock<Result<(), String>> = OnceLock::new();

/// Initializes the store if it is not yet and says whether it is available.
pub fn init() -> bool {
    STORE
        .get_or_init(|| {
            let result = open_store();
            if let Err(e) = &result {
                log::warn!("System keyring not available: {e}");
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

/// Availability without blocking: `None` until it has been checked.
pub fn available() -> Option<bool> {
    STORE.get().map(Result::is_ok)
}

fn entry(id: &str, label: &str) -> Result<Entry, String> {
    if !init() {
        return Err(t!("keyring-unavailable"));
    }
    // Only Secret Service supports a label (the one shown in Seahorse/KWallet).
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

/// Password saved under `id`, or `None` if there is none.
pub fn get(id: &str) -> Result<Option<String>, String> {
    match entry(id, SERVICE)?.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(Error::NoEntry) => Ok(None),
        Err(e) => Err(describe(e)),
    }
}

/// Saves (or replaces) the password of `id`. `label` is only used when creating it.
pub fn set(id: &str, label: &str, password: &str) -> Result<(), String> {
    entry(id, label)?.set_password(password).map_err(describe)
}

/// Deletes the password of `id`; it is not an error if it does not exist.
pub fn delete(id: &str) -> Result<(), String> {
    match entry(id, SERVICE)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(e) => Err(describe(e)),
    }
}

/// `delete` on its own thread, for the UI: a failure is only logged.
pub fn delete_in_background(id: String) {
    std::thread::spawn(move || {
        if let Err(e) = delete(&id) {
            log::warn!("Could not delete the password from the keyring: {e}");
        }
    });
}

fn describe(e: Error) -> String {
    match e {
        Error::NoStorageAccess(err) => {
            t!("keyring-locked", error = err.to_string())
        }
        e => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uses the real system keyring: `cargo test -- --ignored secrets`.
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
