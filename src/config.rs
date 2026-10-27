use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ServerConfig {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub local_folders: Vec<String>,
    #[serde(default)]
    pub servers: Vec<ServerConfig>,
    #[serde(default = "default_volume")]
    pub volume: f32,
}

fn default_volume() -> f32 {
    0.8
}

impl Default for Config {
    fn default() -> Self {
        Self {
            local_folders: Vec::new(),
            servers: Vec::new(),
            volume: default_volume(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|err| {
                eprintln!("config inválida em {}: {err}", path.display());
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("criando diretório {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self).context("serializando configuração")?;
        fs::write(path, text).with_context(|| format!("gravando {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn next_server_id(&self) -> i64 {
        self.servers.iter().map(|s| s.id).max().unwrap_or(0) + 1
    }
}

pub struct Dirs {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
}

impl Dirs {
    pub fn resolve() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let config = dirs::config_dir()
            .unwrap_or_else(|| home.join(".config"))
            .join("aural");
        let data = dirs::data_dir()
            .unwrap_or_else(|| home.join(".local/share"))
            .join("aural");
        let cache = dirs::cache_dir()
            .unwrap_or_else(|| home.join(".cache"))
            .join("aural");
        for dir in [&config, &data, &cache] {
            let _ = fs::create_dir_all(dir);
        }
        Self {
            config,
            data,
            cache,
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.config.join("config.toml")
    }

    pub fn db_file(&self) -> PathBuf {
        self.data.join("library.db")
    }

    pub fn covers_dir(&self) -> PathBuf {
        self.cache.join("covers")
    }
}
