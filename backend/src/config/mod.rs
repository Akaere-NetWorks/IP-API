use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub app: AppConfig,
    pub maxmind: MaxmindConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    pub name: String,
    pub port: u16,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MaxmindConfig {
    pub account_id: u64,
    pub license_key: String,
    pub update_interval_hours: u64,
    pub download_urls: MaxmindUrls,
    pub database_dir: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MaxmindUrls {
    pub asn: String,
    pub city: String,
    pub country: String,
}

impl Config {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Arc<Config>, String> {
        let mut file = File::open(path).map_err(|e| format!("打开配置文件失败: {}", e))?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .map_err(|e| format!("读取配置文件失败: {}", e))?;

        let config: Config = serde_yaml::from_str(&contents)
            .map_err(|e| format!("解析配置文件失败: {}", e))?;

        Ok(Arc::new(config))
    }
}

pub fn init() -> Result<Arc<Config>, String> {
    Config::load("config.yaml")
} 