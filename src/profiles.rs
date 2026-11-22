use serde::{Deserialize, Serialize};

use crate::db::{ConnectionParams, Engine, TlsMode};
use crate::i18n::t;

/// Identifying colour of a connection: tints its tab and its sidebar tree.
/// The presets are resolved in theme.rs with light/dark variants.
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
    /// sRGB picked with the free picker (the same in both themes).
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

    pub fn label(self) -> String {
        match self {
            ConnColor::Blue => t!("color-blue"),
            ConnColor::Teal => t!("color-teal"),
            ConnColor::Green => t!("color-green"),
            ConnColor::Yellow => t!("color-yellow"),
            ConnColor::Orange => t!("color-orange"),
            ConnColor::Red => t!("color-red"),
            ConnColor::Pink => t!("color-pink"),
            ConnColor::Purple => t!("color-purple"),
            ConnColor::Custom(_) => t!("color-custom"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionProfile {
    /// Stable identifier: key of the password in the keyring (the name may
    /// change). Profiles from earlier versions get one when loaded.
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// Server engine. Profiles from before PostgreSQL are MySQL ones.
    #[serde(default)]
    pub engine: Engine,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// Database the connection is limited to; empty = all of them.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub database: String,
    /// Connection security; by default, encrypt if possible.
    #[serde(default, skip_serializing_if = "is_default")]
    pub tls: TlsMode,
    /// CA file to verify the server with; empty = public authorities.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tls_ca: String,
    /// The password is in the system keyring, under `id`.
    #[serde(default)]
    pub password_in_keyring: bool,
    /// Plain-text password from earlier versions. On startup it is moved to
    /// the keyring; it is only kept if no keyring is available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// None = no colour (neutral tab and sidebar)
    #[serde(default)]
    pub color: Option<ConnColor>,
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

impl ConnectionProfile {
    pub fn to_params(&self) -> ConnectionParams {
        ConnectionParams {
            engine: self.engine,
            host: self.host.clone(),
            port: self.port,
            user: self.user.clone(),
            password: self.password.clone().unwrap_or_default(),
            keyring_id: self.password_in_keyring.then(|| self.id.clone()),
            database: self.database.clone(),
            tls: self.tls,
            tls_ca: self.tls_ca.clone(),
        }
    }

    /// Where it connects to, in one line: "PostgreSQL · app@db:5432 · shop".
    pub fn summary(&self) -> String {
        let mut text = format!(
            "{} · {}@{}:{}",
            self.engine.label(),
            self.user,
            self.host,
            self.port
        );
        if !self.database.is_empty() {
            text.push_str(" · ");
            text.push_str(&self.database);
        }
        if self.tls != TlsMode::default() {
            text.push_str(" · ");
            text.push_str(&self.tls.label());
        }
        text
    }

    /// Has a saved password, in the keyring or (inherited) in plain text.
    pub fn has_password(&self) -> bool {
        self.password_in_keyring || self.password.is_some()
    }

    /// Name of its keyring entry (the one Seahorse or KWallet shows).
    pub fn keyring_label(&self) -> String {
        format!("RustQL — {}", self.name)
    }
}

/// New identifier for a profile (random).
pub fn new_id() -> String {
    use std::hash::{BuildHasher, RandomState};
    format!(
        "{:016x}",
        RandomState::new().hash_one(std::time::SystemTime::now())
    )
}

/// Plain-text passwords from earlier versions, to move them to the keyring:
/// (id, label, password).
pub fn legacy_passwords(profiles: &[ConnectionProfile]) -> Vec<(String, String, String)> {
    profiles
        .iter()
        .filter_map(|p| {
            let password = p.password.clone()?;
            Some((p.id.clone(), p.keyring_label(), password))
        })
        .collect()
}

const PROFILES_FILE: &str = "connections.json";

pub fn load() -> Vec<ConnectionProfile> {
    let mut profiles: Vec<ConnectionProfile> = crate::config_file::load(PROFILES_FILE);
    if normalize(&mut profiles) {
        save(&profiles);
    }
    profiles
}

/// Completes profiles from earlier versions: gives them an `id` and drops
/// an empty password (the same as having none). Returns whether any changed.
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

/// Readable only by the user: it may contain inherited plain-text
/// passwords (when no keyring is available).
pub fn save(profiles: &[ConnectionProfile]) {
    crate::config_file::save(PROFILES_FILE, &profiles, true);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfil_sin_color_carga_como_none() {
        // connections.json from before connection colours
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
        // an empty one is the same as having no password
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
            engine: Engine::Mysql,
            host: "db".into(),
            port: 3306,
            user: "app".into(),
            database: String::new(),
            tls: TlsMode::default(),
            tls_ca: String::new(),
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
    fn los_perfiles_anteriores_son_de_mysql() {
        let json = r#"[{"name":"Local","host":"127.0.0.1","port":3306,"user":"root"}]"#;
        let mut profiles: Vec<ConnectionProfile> = serde_json::from_str(json).unwrap();
        assert_eq!(profiles[0].to_params().engine, Engine::Mysql);
        assert_eq!(
            profiles[0].summary(),
            "MySQL / MariaDB · root@127.0.0.1:3306"
        );

        profiles[0].engine = Engine::Postgres;
        profiles[0].database = "tienda".into();
        let json = serde_json::to_string(&profiles[0]).unwrap();
        assert!(json.contains(r#""engine":"postgres""#), "{json}");
        let back: ConnectionProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.to_params().engine, Engine::Postgres);
        assert_eq!(back.summary(), "PostgreSQL · root@127.0.0.1:3306 · tienda");
    }

    #[test]
    fn la_base_de_datos_del_perfil_es_opcional() {
        // Older profiles do not have it: they see the whole server
        let json = r#"[{"name":"Local","host":"127.0.0.1","port":3306,"user":"root"}]"#;
        let mut profiles: Vec<ConnectionProfile> = serde_json::from_str(json).unwrap();
        assert_eq!(profiles[0].to_params().only_database(), None);
        assert!(
            !serde_json::to_string(&profiles[0])
                .unwrap()
                .contains("database")
        );

        profiles[0].database = "tienda".into();
        let json = serde_json::to_string(&profiles[0]).unwrap();
        let back: ConnectionProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.to_params().only_database(), Some("tienda"));
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
