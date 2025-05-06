use serde::{Deserialize, Serialize};
use reqwest::Client;
use std::time::Duration;
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BgpApiMeta {
    #[serde(rename = "sourceType")]
    pub source_type: Option<String>,
    #[serde(rename = "sourceID")]
    pub source_id: Option<String>,
    #[serde(rename = "originASNs")]
    pub origin_asns: Option<Vec<String>>,
    pub r#type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BgpApiResult {
    pub prefix: String,
    pub meta: Vec<BgpApiMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BgpApiResponse {
    pub r#type: String,
    pub prefix: String,
    pub result: Option<BgpApiResult>,
}

pub struct BgpApiClient;

impl BgpApiClient {
    pub async fn query(ip: &str) -> Result<BgpApiResult, String> {
        // 根据 IP 类型添加默认掩码（IPv4: /32, IPv6: /128）
        let prefix = if ip.contains(':') {
            format!("{}/128", ip)
        } else {
            format!("{}/32", ip)
        };
        let url = format!("https://rest.bgp-api.net/api/v1/prefix/{}/search", prefix);
        info!("BGP API 请求 URL: {}", url);
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| format!("创建HTTP客户端失败: {}", e))?;

        let resp = client.get(&url).send().await
            .map_err(|e| format!("BGP-API请求失败: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("BGP-API请求失败: 状态码 {}", resp.status()));
        }

        let json: BgpApiResponse = resp.json().await
            .map_err(|e| format!("解析BGP-API响应失败: {}", e))?;

        if let Some(result) = json.result {
            Ok(result)
        } else {
            Err("BGP-API响应无result".to_string())
        }
    }
} 