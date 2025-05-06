use crate::config::MaxmindConfig;
use chrono::{DateTime, Utc};
use log::{info, warn, error, debug};
use reqwest::Client;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

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
        self.download_and_extract_database("asn").await?;
        self.download_and_extract_database("city").await?;
        self.download_and_extract_database("country").await?;
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

    async fn download_and_extract_database(&self, db_type: &str) -> Result<(), String> {
        let url = self.get_download_url(db_type)?;
        info!("准备下载 {} 数据库: {}", db_type, url);
        let account_id = self.config.account_id.to_string();
        let license_key = self.config.license_key.clone();
        let mut last_err = None;
        for attempt in 1..=3 {
            info!("第{}次尝试下载 {} 数据库...", attempt, db_type);
            let response = self.client
                .get(&url)
                .basic_auth(account_id.clone(), Some(license_key.clone()))
                .send()
                .await;
            match response {
                Ok(resp) => {
                    debug!("{} 数据库响应状态: {}", db_type, resp.status());
                    if !resp.status().is_success() {
                        last_err = Some(format!("下载 {} 数据库失败: HTTP状态码 {}", db_type, resp.status()));
                        warn!("第{}次尝试失败，状态码: {}，重试...", attempt, resp.status());
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue;
                    }
                    let content = resp.bytes().await.map_err(|e| format!("读取 {} 数据库响应失败: {}", db_type, e))?;
                    info!("{} 数据库下载完成，大小: {} 字节，开始解压...", db_type, content.len());
                    let db_type_owned = db_type.to_string();
                    match self.extract_tar_gz(content.to_vec(), db_type_owned.clone()).await {
                        Ok(_) => {
                            info!("成功更新 {} 数据库", db_type_owned);
                            return Ok(());
                        },
                        Err(e) => {
                            error!("解压 {} 数据库失败: {}", db_type_owned, e);
                            last_err = Some(format!("解压 {} 数据库失败: {}", db_type_owned, e));
                            // 不重试解压，直接返回
                            return Err(last_err.unwrap());
                        }
                    }
                }
                Err(e) => {
                    last_err = Some(format!("下载 {} 数据库失败: {}", db_type, e));
                    warn!("第{}次尝试失败，错误: {}，重试...", attempt, e);
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
        error!("{} 数据库下载失败: {:?}", db_type, last_err);
        Err(last_err.unwrap_or_else(|| format!("下载 {} 数据库失败: 未知错误", db_type)))
    }

    fn get_download_url(&self, db_type: &str) -> Result<String, String> {
        let url = match db_type {
            "asn" => &self.config.download_urls.asn,
            "city" => &self.config.download_urls.city,
            "country" => &self.config.download_urls.country,
            _ => return Err(format!("无效的数据库类型: {}", db_type)),
        };
        Ok(url.clone())
    }

    async fn extract_tar_gz(&self, data: Vec<u8>, db_type: String) -> Result<(), String> {
        use std::fs::File;
        info!("解压 {} 数据库，写入临时文件...", db_type);
        let temp_dir = tempfile::Builder::new().prefix("maxmind").tempdir()
            .map_err(|e| format!("创建临时目录失败: {}", e))?;
        let tar_path = temp_dir.path().join(format!("{}.tar.gz", &db_type));
        let mut file = tokio::fs::File::create(&tar_path)
            .await
            .map_err(|e| format!("创建临时文件失败: {}", e))?;
        file.write_all(&data)
            .await
            .map_err(|e| format!("写入临时文件失败: {}", e))?;
        file.flush()
            .await
            .map_err(|e| format!("刷新临时文件失败: {}", e))?;
        drop(file);
        info!("{} 数据库临时文件写入完成: {}，开始解压...", db_type, tar_path.display());
        let temp_dir_path = temp_dir.path().to_path_buf();
        let tar_path_clone = tar_path.clone();
        let db_dir = self.config.database_dir.clone();
        let db_type_clone = db_type.clone();
        let result = tokio::task::spawn_blocking(move || {
            info!("[阻塞线程] 打开tar.gz文件: {}", tar_path_clone.display());
            let tar_file = match File::open(&tar_path_clone) {
                Ok(f) => f,
                Err(e) => return Err(format!("打开临时文件失败: {}", e)),
            };
            info!("[阻塞线程] 解压tar.gz...");
            let tar = flate2::read::GzDecoder::new(tar_file);
            let mut archive = tar::Archive::new(tar);
            if let Err(e) = archive.unpack(&temp_dir_path) {
                return Err(format!("解压数据库失败: {}", e));
            }
            let db_file_name = format!("GeoLite2-{}.mmdb", 
                db_type_clone.chars().next().unwrap().to_uppercase().collect::<String>() + &db_type_clone[1..]);
            info!("[阻塞线程] 查找解压后的mmdb文件(忽略大小写): {}", db_file_name);
            let mut db_file_path = None;
            for entry in walkdir::WalkDir::new(&temp_dir_path).into_iter().filter_map(|e| e.ok()) {
                let file_name = entry.file_name().to_string_lossy();
                info!("[阻塞线程] 解压内容: {}", entry.path().display());
                if file_name.to_lowercase().ends_with(&db_file_name.to_lowercase()) {
                    db_file_path = Some(entry.path().to_path_buf());
                    break;
                }
            }
            let db_path = match db_file_path {
                Some(p) => p,
                None => return Err(format!("在解压后的文件中未找到 {} 数据库文件", db_type_clone)),
            };
            Ok((db_path, db_file_name))
        }).await.map_err(|e| format!("解压任务失败: {}", e))??;
        let (db_file_path, db_file_name) = result;
        info!("复制mmdb文件到目标目录: {}", db_file_name);
        let target_path = Path::new(&db_dir).join(&db_file_name);
        tokio::fs::copy(db_file_path, &target_path)
            .await
            .map_err(|e| format!("复制数据库文件失败: {}", e))?;
        info!("成功提取并保存 {} 数据库到 {}", db_type, target_path.display());
        Ok(())
    }
} 