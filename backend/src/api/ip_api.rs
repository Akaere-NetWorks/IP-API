use crate::maxmind::reader::MaxmindReader;
use crate::utils::ip_cache::IpCache;
use crate::utils::whois_client::WhoisClient;
use crate::utils::bgptools_client::{BgpToolsClient, BgpToolsUpstream};
use crate::utils::rpki_client::{RpkiClient, RpkiValidity};
use crate::utils::bgp_api_client::BgpApiClient;
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
use tracing::{info, warn, debug};
use futures::future::join_all;

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
pub struct BgpInfoResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allocated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_name: Option<String>,
    pub upstreams: Vec<BgpToolsUpstream>,
}

#[derive(Serialize, Deserialize)]
pub struct IpResponse {
    pub info: IpInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub whois_info: Option<WhoisInfoResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bgp_info: Option<BgpInfoResponse>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rpki_info_list: Vec<RpkiValidity>,
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
                // 并发请求所有后端信息
                let ip_cloned = ip.clone();
                let whois_future = async {
                    if info.whois_info.is_none() {
                        match WhoisClient::lookup(&ip_cloned) {
                            Ok(whois_info) => Some(whois_info),
                            Err(e) => {
                                warn!("获取WHOIS信息失败 {}: {}", ip_cloned, e);
                                None
                            }
                        }
                    } else {
                        None
                    }
                };
                
                let bgp_tools_future = async {
                    if info.bgp_info.is_none() {
                        match BgpToolsClient::lookup(&ip_cloned).await {
                            Ok(bgp_info) => Some(bgp_info),
                            Err(e) => {
                                warn!("获取BGP Tools信息失败 {}: {}", ip_cloned, e);
                                None
                            }
                        }
                    } else {
                        None
                    }
                };
                
                let bgp_api_future = async {
                    if info.bgp_api_info.is_none() {
                        match BgpApiClient::query(&ip_cloned).await {
                            Ok(bgp_result) => Some(bgp_result),
                            Err(e) => {
                                warn!("获取BGP API信息失败 {}: {}", ip_cloned, e);
                                debug!("获取BGP API信息失败详情 {}: {:?}", ip_cloned, e);
                                None
                            }
                        }
                    } else {
                        None
                    }
                };
                
                // 并发执行所有请求
                let (whois_result, bgp_tools_result, bgp_api_result) = tokio::join!(
                    whois_future,
                    bgp_tools_future,
                    bgp_api_future
                );
                
                // 处理查询结果
                if let Some(whois_info) = whois_result {
                    info.whois_info = Some(whois_info);
                }
                
                if let Some(bgp_info) = bgp_tools_result {
                    info.bgp_info = Some(bgp_info);
                }
                
                if let Some(bgp_result) = bgp_api_result {
                    info.bgp_api_info = Some(bgp_result.clone());
                    
                    // 处理RPKI查询
                    if let Some(meta) = info.bgp_api_info.as_ref().unwrap().meta.iter().find(|m| m.origin_asns.is_some()) {
                        if let (Some(prefix), Some(asns)) = (Some(&info.bgp_api_info.as_ref().unwrap().prefix), &meta.origin_asns) {
                            info!("准备执行RPKI查询, prefix={}, ASNs={:?}", prefix, asns);
                            
                            // 并发查询所有ASN的RPKI信息
                            let rpki_futures = asns.iter().map(|asn| {
                                let prefix = prefix.clone();
                                let asn = asn.clone();
                                async move {
                                    let rpki_client = RpkiClient::new("http://rpki.akae.re");
                                    info!("发送RPKI请求: prefix={}, asn={}", prefix, asn);
                                    match rpki_client.query(&prefix, &asn).await {
                                        Ok(validity) => Some(validity),
                                        Err(e) => {
                                            warn!("RPKI查询失败 {}: {}", asn, e);
                                            None
                                        }
                                    }
                                }
                            }).collect::<Vec<_>>();
                            
                            // 等待所有RPKI查询完成
                            let rpki_results = join_all(rpki_futures).await;
                            
                            // 收集有效的RPKI结果
                            info.rpki_info_list = rpki_results
                                .into_iter()
                                .filter_map(|r| r)
                                .collect();
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
        let mut bgp_info = None;
        
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
        
        // 添加BGP Tools信息（如果有）
        if let Some(bgp) = &info.bgp_info {
            bgp_info = Some(BgpInfoResponse {
                asn: bgp.asn.clone(),
                prefix: bgp.prefix.clone(),
                country: bgp.country.clone(),
                registry: bgp.registry.clone(),
                allocated: bgp.allocated.clone(),
                as_name: bgp.as_name.clone(),
                upstreams: bgp.upstreams.clone(),
            });
        }
        
        IpResponse {
            info: ip_info,
            whois_info,
            bgp_info,
            rpki_info_list: info.rpki_info_list.clone(),
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