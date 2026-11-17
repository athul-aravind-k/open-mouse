use crate::edge::{DisplayArrangement, EdgeDirection};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub fn generate_machine_id() -> String {
    let hostname = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Host".to_string());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}_{:016x}", hostname, nanos)
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct AppConfig {
    #[serde(default = "generate_machine_id")]
    pub machine_id: String,
    pub device_name: String,
    pub peer_ip: String,
    pub peer_port: u16,
    pub shared_edge: EdgeDirection,
    #[serde(default)]
    pub display_arrangement: DisplayArrangement,
    #[serde(default)]
    pub trusted_fingerprints: Vec<String>,
    pub autostart: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        let hostname = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Host".to_string());
        Self {
            machine_id: generate_machine_id(),
            device_name: hostname,
            peer_ip: String::new(),
            peer_port: 48899,
            shared_edge: EdgeDirection::Right,
            display_arrangement: DisplayArrangement::default(),
            trusted_fingerprints: Vec::new(),
            autostart: false,
        }
    }
}

pub fn get_config_dir() -> PathBuf {
    if let Ok(app_data) = std::env::var("APPDATA") {
        PathBuf::from(app_data).join("OpenMouse")
    } else {
        PathBuf::from(".").join(".openmouse")
    }
}

pub fn get_config_path() -> PathBuf {
    get_config_dir().join("config.json")
}

pub fn load_config() -> AppConfig {
    let path = get_config_path();
    if path.exists() {
        if let Ok(contents) = fs::read_to_string(&path) {
            if let Ok(config) = serde_json::from_str::<AppConfig>(&contents) {
                return config;
            }
        }
    }
    let default = AppConfig::default();
    let _ = save_config(&default);
    default
}

pub fn save_config(config: &AppConfig) -> std::io::Result<()> {
    let dir = get_config_dir();
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
    }
    let path = get_config_path();
    let json = serde_json::to_string_pretty(config)?;
    fs::write(path, json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_roundtrip() {
        let config = AppConfig {
            machine_id: "test-machine-id".to_string(),
            device_name: "TestLaptop".to_string(),
            peer_ip: "192.168.1.50".to_string(),
            peer_port: 48899,
            shared_edge: EdgeDirection::Right,
            display_arrangement: DisplayArrangement {
                relative_edge: EdgeDirection::Right,
                offset_x: 0,
                offset_y: 120,
            },
            trusted_fingerprints: vec!["AA:BB:CC".to_string()],
            autostart: true,
        };
        let serialized = serde_json::to_string(&config).unwrap();
        let deserialized: AppConfig = serde_json::from_str(&serialized).unwrap();
        assert_eq!(config, deserialized);
    }
}

