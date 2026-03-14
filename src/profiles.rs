use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::db::ConnectionParams;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionProfile {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// None = no guardar contraseña
    pub password: Option<String>,
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
