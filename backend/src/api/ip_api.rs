use crate::maxmind::reader::MaxmindReader;
use crate::utils::ip_cache::IpCache;
use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Json},
    Router,
    routing::get,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;

#[derive(Serialize, Deserialize)]
pub struct IpResponse {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached: Option<bool>,
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
        // 首先尝试从缓存获取
        if let Some(cached_info) = state.cache.get(&ip).await {
            info!("从缓存获取IP信息: {}", ip);
            let response = IpResponse {
                ip: cached_info.ip,
                ip_range: cached_info.ip_range,
                country: cached_info.country,
                city: cached_info.city,
                asn: cached_info.asn,
                organization: cached_info.organization,
                cached: Some(true),
            };
            
            return (StatusCode::OK, Json(response)).into_response();
        }
        
        // 缓存未命中，从MaxMind查询
        let reader = state.reader.read().await;
        
        match reader.lookup(&ip) {
            Ok(info) => {
                // 构建响应
                let response = IpResponse {
                    ip: info.ip.clone(),
                    ip_range: info.ip_range.clone(),
                    country: info.country.clone(),
                    city: info.city.clone(),
                    asn: info.asn.clone(),
                    organization: info.organization.clone(),
                    cached: Some(false),
                };
                
                // 将结果存入缓存
                if let Err(e) = state.cache.set(&ip, info).await {
                    tracing::warn!("无法缓存IP信息 {}: {}", ip, e);
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