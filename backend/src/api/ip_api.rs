use crate::maxmind::reader::MaxmindReader;
use crate::utils::ip_cache::IpCache;
use crate::utils::whois_client::WhoisClient;
use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Json},
    Router,
    routing::get,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

#[derive(Serialize, Deserialize)]
pub struct IpInfo {
    pub ip: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip_range: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asn: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct WhoisInfoResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub netname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maintainer: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct IpResponse {
    pub info: IpInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub whois_info: Option<WhoisInfoResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached: Option<u64>, // 缓存时间戳，如果不是缓存则为None
}

#[derive(Serialize, Deserialize)]
pub struct ErrorResponse {
    pub status: String,
    pub message: String,
}

pub struct IpApiHandler {
    reader: Arc<tokio::sync::RwLock<MaxmindReader>>,
    cache: Arc<IpCache>,
}

impl IpApiHandler {
    pub fn new(reader: Arc<tokio::sync::RwLock<MaxmindReader>>, cache: Arc<IpCache>) -> Self {
        Self { reader, cache }
    }

    pub fn router(self) -> Router {
        Router::new()
            .route("/ip/:ip", get(Self::get_ip_info))
            .route("/stats/cache", get(Self::get_cache_stats))
            .with_state(Arc::new(self))
    }

    async fn get_ip_info(
        Path(ip): Path<String>,
        axum::extract::State(state): axum::extract::State<Arc<Self>>,
    ) -> impl IntoResponse {
        // 获取当前时间戳
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
            
        // 首先尝试从缓存获取
        if let Some(cached_info) = state.cache.get(&ip).await {
            info!("从缓存获取IP信息: {}", ip);
            let response = Self::create_response_from_ip_info(&cached_info, Some(now));
            return (StatusCode::OK, Json(response)).into_response();
        }
        
        // 缓存未命中，从MaxMind查询
        let reader = state.reader.read().await;
        
        match reader.lookup(&ip) {
            Ok(mut info) => {
                // 如果没有WHOIS信息，尝试获取
                if info.whois_info.is_none() {
                    match WhoisClient::lookup(&ip) {
                        Ok(whois_info) => {
                            info.whois_info = Some(whois_info);
                        }
                        Err(e) => {
                            warn!("获取WHOIS信息失败 {}: {}", ip, e);
                        }
                    }
                }
                
                // 构建响应
                let response = Self::create_response_from_ip_info(&info, None);
                
                // 将结果存入缓存
                if let Err(e) = state.cache.set(&ip, info).await {
                    warn!("无法缓存IP信息 {}: {}", ip, e);
                }
                
                (StatusCode::OK, Json(response)).into_response()
            },
            Err(e) => {
                let response = ErrorResponse {
                    status: "error".to_string(),
                    message: e,
                };
                
                (StatusCode::BAD_REQUEST, Json(response)).into_response()
            }
        }
    }
    
    fn create_response_from_ip_info(info: &crate::maxmind::reader::IpInfo, cached_timestamp: Option<u64>) -> IpResponse {
        let ip_info = IpInfo {
            ip: info.ip.clone(),
            ip_range: info.ip_range.clone(),
            country: info.country.clone(),
            city: info.city.clone(),
            asn: info.asn,
            organization: info.organization.clone(),
        };
        
        let mut whois_info = None;
        
        // 添加WHOIS信息（如果有）
        if let Some(whois) = &info.whois_info {
            whois_info = Some(WhoisInfoResponse {
                netname: whois.netname.clone(),
                descr: whois.descr.clone(),
                country: whois.country.clone(),
                org: whois.org.clone(),
                admin: whois.admin_c.clone(),
                maintainer: whois.mnt_by.clone(),
            });
        }
        
        IpResponse {
            info: ip_info,
            whois_info,
            cached: cached_timestamp,
        }
    }
    
    async fn get_cache_stats(
        axum::extract::State(state): axum::extract::State<Arc<Self>>,
    ) -> impl IntoResponse {
        let (entries, memory_mb) = state.cache.stats().await;
        
        #[derive(Serialize)]
        struct CacheStats {
            entries: usize,
            memory_mb: f64,
        }
        
        let stats = CacheStats {
            entries,
            memory_mb,
        };
        
        (StatusCode::OK, Json(stats)).into_response()
    }
} 