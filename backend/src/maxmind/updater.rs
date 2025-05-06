use crate::config::MaxmindConfig;
use chrono::{DateTime, Utc};
use log::info;
use reqwest::blocking::Client;
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

pub struct MaxmindUpdater {
    config: Arc<MaxmindConfig>,
    client: Client,
    last_update: Option<DateTime<Utc>>,
}

impl MaxmindUpdater {
    pub fn new(config: Arc<MaxmindConfig>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .expect("构建HTTP客户端失败");

        Self {
            config,
            client,
            last_update: None,
        }
    }

    pub async fn update(&mut self) -> Result<(), String> {
        info!("开始更新MaxMind数据库...");
        
        self.ensure_database_dir()?;
        
        self.download_and_extract_database("asn")?;
        self.download_and_extract_database("city")?;
        self.download_and_extract_database("country")?;
        
        self.last_update = Some(Utc::now());
        info!("MaxMind数据库更新完成");
        
        Ok(())
    }
    
    fn ensure_database_dir(&self) -> Result<(), String> {
        let path = Path::new(&self.config.database_dir);
        if !path.exists() {
            fs::create_dir_all(path).map_err(|e| format!("创建数据库目录失败: {}", e))?;
        }
        Ok(())
    }

    fn download_and_extract_database(&self, db_type: &str) -> Result<(), String> {
        let url = self.get_download_url(db_type)?;
        info!("下载 {} 数据库...", db_type);
        
        let response = self.client
            .get(&url)
            .send()
            .map_err(|e| format!("下载 {} 数据库失败: {}", db_type, e))?;
        
        if !response.status().is_success() {
            return Err(format!("下载 {} 数据库失败: HTTP状态码 {}", db_type, response.status()));
        }
        
        let content = response.bytes()
            .map_err(|e| format!("读取 {} 数据库响应失败: {}", db_type, e))?;
        
        info!("提取 {} 数据库...", db_type);
        self.extract_tar_gz(content.as_ref(), db_type)
    }
    
    fn get_download_url(&self, db_type: &str) -> Result<String, String> {
        let url = match db_type {
            "asn" => &self.config.download_urls.asn,
            "city" => &self.config.download_urls.city,
            "country" => &self.config.download_urls.country,
            _ => return Err(format!("无效的数据库类型: {}", db_type)),
        };
        
        Ok(url.replace("{license_key}", &self.config.license_key))
    }
    
    fn extract_tar_gz(&self, data: &[u8], db_type: &str) -> Result<(), String> {
        let cursor = Cursor::new(data);
        let tar = flate2::read::GzDecoder::new(cursor);
        let mut archive = tar::Archive::new(tar);
        
        let temp_dir = tempfile::Builder::new().prefix("maxmind").tempdir()
            .map_err(|e| format!("创建临时目录失败: {}", e))?;
        
        archive.unpack(&temp_dir)
            .map_err(|e| format!("解压数据库失败: {}", e))?;
        
        let db_file_name = format!("GeoLite2-{}.mmdb", db_type.chars().next().unwrap().to_uppercase().collect::<String>() + &db_type[1..]);
        let mut found = false;
        
        for entry in walkdir::WalkDir::new(&temp_dir) {
            let entry = entry.map_err(|e| format!("遍历解压文件失败: {}", e))?;
            
            if entry.file_name().to_string_lossy().ends_with(&db_file_name) {
                let target_path = Path::new(&self.config.database_dir).join(&db_file_name);
                fs::copy(entry.path(), &target_path)
                    .map_err(|e| format!("复制数据库文件失败: {}", e))?;
                    
                info!("成功提取并保存 {} 数据库到 {}", db_type, target_path.display());
                found = true;
                break;
            }
        }
        
        if !found {
            return Err(format!("在解压后的文件中未找到 {} 数据库文件", db_type));
        }
        
        Ok(())
    }
} 