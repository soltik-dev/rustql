use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::db::ConnectionParams;

/// Color identificativo de una conexión: tiñe su pestaña y su árbol del sidebar.
/// Los presets se resuelven en theme.rs con variante clara/oscura.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnColor {
    Blue,
    Teal,
    Green,
    Yellow,
    Orange,
    Red,
    Pink,
    Purple,
    /// sRGB elegido con el selector libre (igual en ambos temas).
    Custom([u8; 3]),
}

impl ConnColor {
    pub const PRESETS: [ConnColor; 8] = [
        ConnColor::Blue,
        ConnColor::Teal,
        ConnColor::Green,
        ConnColor::Yellow,
        ConnColor::Orange,
        ConnColor::Red,
        ConnColor::Pink,
        ConnColor::Purple,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ConnColor::Blue => "Azul",
            ConnColor::Teal => "Turquesa",
            ConnColor::Green => "Verde",
            ConnColor::Yellow => "Amarillo",
            ConnColor::Orange => "Naranja",
            ConnColor::Red => "Rojo",
            ConnColor::Pink => "Rosa",
            ConnColor::Purple => "Morado",
            ConnColor::Custom(_) => "Personalizado",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionProfile {
    /// Identificador estable: clave de la contraseña en el llavero (el nombre
    /// puede cambiar). Los perfiles de versiones anteriores lo reciben al cargar.
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// La contraseña está en el llavero del sistema, bajo `id`.
    #[serde(default)]
    pub password_in_keyring: bool,
    /// Contraseña en claro de versiones anteriores. Al arrancar se pasa al
    /// llavero; solo se conserva si no hay llavero disponible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// None = sin color (pestaña y sidebar neutros)
    #[serde(default)]
    pub color: Option<ConnColor>,
}

impl ConnectionProfile {
    pub fn to_params(&self) -> ConnectionParams {
        ConnectionParams {
            host: self.host.clone(),
            port: self.port,
            user: self.user.clone(),
            password: self.password.clone().unwrap_or_default(),
            keyring_id: self.password_in_keyring.then(|| self.id.clone()),
        }
    }

    /// Tiene contraseña guardada, en el llavero o (heredada) en claro.
    pub fn has_password(&self) -> bool {
        self.password_in_keyring || self.password.is_some()
    }

    /// Nombre de su entrada en el llavero (el que muestran Seahorse o KWallet).
    pub fn keyring_label(&self) -> String {
        format!("RustQL — {}", self.name)
    }
}

/// Identificador nuevo para un perfil (aleatorio).
pub fn new_id() -> String {
    use std::hash::{BuildHasher, RandomState};
    format!(
        "{:016x}",
        RandomState::new().hash_one(std::time::SystemTime::now())
    )
}

/// Contraseñas en claro de versiones anteriores, para pasarlas al llavero:
/// (id, etiqueta, contraseña).
pub fn legacy_passwords(profiles: &[ConnectionProfile]) -> Vec<(String, String, String)> {
    profiles
        .iter()
        .filter_map(|p| {
            let password = p.password.clone()?;
            Some((p.id.clone(), p.keyring_label(), password))
        })
        .collect()
}

fn profiles_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("rustql").join("connections.json"))
}

pub fn load() -> Vec<ConnectionProfile> {
    let Some(path) = profiles_path() else {
        return vec![];
    };
    let Ok(data) = std::fs::read_to_string(path) else {
        return vec![];
    };
    let mut profiles: Vec<ConnectionProfile> = serde_json::from_str(&data).unwrap_or_default();
    if normalize(&mut profiles) {
        save(&profiles);
    }
    profiles
}

/// Completa los perfiles de versiones anteriores: les da `id` y descarta la
/// contraseña vacía (equivale a no tenerla). Devuelve si cambió alguno.
fn normalize(profiles: &mut [ConnectionProfile]) -> bool {
    let mut changed = false;
    for p in profiles {
        if p.id.is_empty() {
            p.id = new_id();
            changed = true;
        }
        if p.password.as_deref() == Some("") {
            p.password = None;
            changed = true;
        }
    }
    changed
}

pub fn save(profiles: &[ConnectionProfile]) {
    let Some(path) = profiles_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    if let Ok(data) = serde_json::to_string_pretty(profiles) {
        let _ = write_private(&path, &data);
    }
}

/// Escribe el fichero legible solo por el usuario (0600 en Unix), también si
/// ya existía con otros permisos.
fn write_private(path: &Path, data: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;
    // `mode` solo cuenta al crearlo: corrige los de versiones anteriores.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(data.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfil_sin_color_carga_como_none() {
        // connections.json anterior a los colores de conexión
        let json =
            r#"[{"name":"Local","host":"127.0.0.1","port":3306,"user":"root","password":null}]"#;
        let profiles: Vec<ConnectionProfile> = serde_json::from_str(json).unwrap();
        assert_eq!(profiles[0].color, None);
    }

    #[test]
    fn perfiles_anteriores_reciben_id_y_migran_la_contrasena() {
        let json = r#"[
            {"name":"A","host":"h","port":3306,"user":"root","password":"secreta"},
            {"name":"B","host":"h","port":3306,"user":"root","password":""},
            {"name":"C","host":"h","port":3306,"user":"root","password":null}
        ]"#;
        let mut profiles: Vec<ConnectionProfile> = serde_json::from_str(json).unwrap();
        assert!(normalize(&mut profiles));
        assert!(!normalize(&mut profiles), "segunda pasada sin cambios");

        assert!(profiles.iter().all(|p| p.id.len() == 16));
        assert_ne!(profiles[0].id, profiles[1].id);
        assert_ne!(profiles[1].id, profiles[2].id);
        // la vacía equivale a no tener contraseña
        assert_eq!(profiles[1].password, None);
        assert!(!profiles[1].has_password());

        let legacy = legacy_passwords(&profiles);
        assert_eq!(legacy.len(), 1);
        assert_eq!(legacy[0].0, profiles[0].id);
        assert_eq!(legacy[0].1, "RustQL — A");
        assert_eq!(legacy[0].2, "secreta");
    }

    #[test]
    fn contrasena_del_llavero_no_se_escribe_en_el_json() {
        let profile = ConnectionProfile {
            id: "abc".into(),
            name: "Prod".into(),
            host: "db".into(),
            port: 3306,
            user: "app".into(),
            password_in_keyring: true,
            password: None,
            color: None,
        };
        let json = serde_json::to_string(&profile).unwrap();
        assert!(!json.contains("\"password\""), "{json}");

        let params = profile.to_params();
        assert_eq!(params.password, "");
        assert_eq!(params.keyring_id.as_deref(), Some("abc"));

        let sin_llavero = ConnectionProfile {
            password_in_keyring: false,
            ..profile
        };
        assert_eq!(sin_llavero.to_params().keyring_id, None);
    }

    #[test]
    fn colores_ida_y_vuelta() {
        for color in ConnColor::PRESETS
            .into_iter()
            .chain([ConnColor::Custom([12, 34, 56])])
        {
            let json = serde_json::to_string(&Some(color)).unwrap();
            let back: Option<ConnColor> = serde_json::from_str(&json).unwrap();
            assert_eq!(back, Some(color));
        }
    }
}
