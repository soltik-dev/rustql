use serde::{Deserialize, Serialize};

use crate::db::{ConnectionParams, Engine, TlsMode};

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
    /// Motor del servidor. Los perfiles anteriores a PostgreSQL son de MySQL.
    #[serde(default)]
    pub engine: Engine,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// Base de datos a la que se limita la conexión; vacía = todas.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub database: String,
    /// Seguridad de la conexión; por defecto, cifrar si se puede.
    #[serde(default, skip_serializing_if = "is_default")]
    pub tls: TlsMode,
    /// Fichero de CA para verificar al servidor; vacío = autoridades públicas.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tls_ca: String,
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

    /// A dónde conecta, en una línea: «PostgreSQL · app@db:5432 · tienda».
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
            text.push_str(self.tls.label());
        }
        text
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

const PROFILES_FILE: &str = "connections.json";

pub fn load() -> Vec<ConnectionProfile> {
    let mut profiles: Vec<ConnectionProfile> = crate::config_file::load(PROFILES_FILE);
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

/// Legible solo por el usuario: puede contener contraseñas heredadas en
/// claro (sin llavero disponible).
pub fn save(profiles: &[ConnectionProfile]) {
    crate::config_file::save(PROFILES_FILE, &profiles, true);
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
        // Los perfiles anteriores no la tienen: ven todo el servidor
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
