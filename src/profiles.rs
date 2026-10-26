use std::path::PathBuf;

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
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// None = no guardar contraseña
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
        }
    }
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
    serde_json::from_str(&data).unwrap_or_default()
}

pub fn save(profiles: &[ConnectionProfile]) {
    let Some(path) = profiles_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(data) = serde_json::to_string_pretty(profiles) {
        let _ = std::fs::write(path, data);
    }
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
