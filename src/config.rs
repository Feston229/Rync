use anyhow::{anyhow, Ok, Result};
use std::{env, path::PathBuf};
use tokio::fs;
use toml::Table;

pub const MULTICAST_IP: &str = "239.0.0.1";
pub const MULTICAST_PORT: u16 = 23235;

#[derive(Clone)]
pub struct Config {
    // Directory to synchronize
    // Default: $HOME/Rync
    pub directory: String,
    // Update interval (in secs)
    // Default: 30
    pub interval: u64,
}

impl Config {
    pub async fn init() -> Result<Self> {
        let mut directory: String;
        let interval: u64;
        let config_path = PathBuf::from(env::var("HOME")?)
            .join(".config")
            .join("rync")
            .join("rync.toml");

        if !config_path.exists() {
            directory = env::var("HOME")?;
            directory.push_str("/Rync");

            interval = 30;
        } else {
            let table = fs::read_to_string(&config_path).await?.parse::<Table>()?;
            directory = table["general"]
                .as_table()
                .ok_or(anyhow!("Incorrect config"))?
                .get("directory")
                .ok_or(anyhow!("Incorrect config"))?
                .as_str()
                .ok_or(anyhow!("Incorrect config"))?
                .to_string();
            interval = table["general"]
                .as_table()
                .ok_or(anyhow!("Incorrect config"))?
                .get("interval")
                .ok_or(anyhow!("Incorrect config"))?
                .as_integer()
                .ok_or(anyhow!("Incorrect config"))? as u64;
        }

        if !PathBuf::from(&directory).exists() {
            fs::create_dir_all(&directory).await?;
        }

        let config = Config {
            directory,
            interval,
        };
        Ok(config)
    }
}
