use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub hotkeys: HashMap<String, String>,
    pub mic_device: Option<String>,
    pub paste_mode: Option<String>, // "type" | "paste" | "clipboard"
    pub remote_sizes: HashMap<String, u64>,
}

pub fn app_data_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ThinSTT")
}

pub fn recordings_dir() -> PathBuf {
    app_data_dir().join("recordings")
}

pub fn models_dir() -> PathBuf {
    app_data_dir().join("models")
}

pub fn load() -> Config {
    let path = app_data_dir().join("config.json");
    match fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

pub fn save(cfg: &Config) -> Result<(), String> {
    let dir = app_data_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("config.json");
    let s = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    fs::write(path, s).map_err(|e| e.to_string())
}
