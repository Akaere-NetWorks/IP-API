use serde::{Deserialize, Serialize};
use reqwest::Client;
use std::time::Duration;
use tracing::info;
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiVrps {
    pub asn: String,
    pub prefix: String,
    #[serde(rename = "max_length")]
    pub max_length: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiValidity {
    pub asn: String,
    pub prefix: String,
    pub validity: String,
    pub reason: Option<String>,
    pub vrps: Option<Vec<RpkiVrps>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiResponse {
    pub validated_route: Option<RpkiValidatedRoute>,
    #[serde(rename = "generatedTime")]
    pub generated_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiValidityState {
    pub state: String,
    pub description: String,
    #[serde(rename = "VRPs")]
    pub vrps: Option<RpkiVrpsInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiVrpsInfo {
    pub matched: Vec<RpkiVrps>,
    pub unmatched_as: Vec<Value>,
    pub unmatched_length: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiValidatedRoute {
    pub route: RpkiRoute,
    pub validity: RpkiValidityState,
    #[serde(rename = "VRPs")]
    pub vrps: Option<Vec<RpkiVrps>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiRoute {
    pub origin_asn: String,
    pub prefix: String,
}

pub struct RpkiClient {
    pub base_url: String,
}

impl RpkiClient {
    pub fn new(base_url: &str) -> Self {
        Self { base_url: base_url.trim_end_matches('/').to_string() }
    }

    pub async fn query(&self, prefix: &str, asn: &str) -> Result<RpkiValidity, String> {
        let url = format!("{}/api/v1/validity/{}/{}", self.base_url, asn, prefix);
        info!("RPKI 请求 URL: {}", url);
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("创建HTTP客户端失败: {}", e))?;

        let resp = client.get(&url).send().await
            .map_err(|e| format!("RPKI请求失败: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("RPKI请求失败: 状态码 {}", resp.status()));
        }

        let json: RpkiResponse = resp.json().await
            .map_err(|e| format!("解析RPKI响应失败: {}", e))?;

        if let Some(validated) = json.validated_route {
            Ok(RpkiValidity {
                asn: asn.to_string(),
                prefix: prefix.to_string(),
                validity: validated.validity.state,
                reason: None,
                vrps: validated.vrps,
            })
        } else {
            Ok(RpkiValidity {
                asn: asn.to_string(),
                prefix: prefix.to_string(),
                validity: "not-found".to_string(),
                reason: None,
                vrps: None,
            })
        }
    }
} 