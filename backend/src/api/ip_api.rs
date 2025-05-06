use crate::maxmind::reader::MaxmindReader;
use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Json},
    Router,
    routing::get,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
}

#[derive(Serialize, Deserialize)]
pub struct ErrorResponse {
    pub status: String,
    pub message: String,
}

pub struct IpApiHandler {
    reader: Arc<tokio::sync::RwLock<MaxmindReader>>,
}

impl IpApiHandler {
    pub fn new(reader: Arc<tokio::sync::RwLock<MaxmindReader>>) -> Self {
        Self { reader }
    }

    pub fn router(self) -> Router {
        Router::new()
            .route("/ip/:ip", get(Self::get_ip_info))
            .with_state(Arc::new(self))
    }

    async fn get_ip_info(
        Path(ip): Path<String>,
        axum::extract::State(state): axum::extract::State<Arc<Self>>,
    ) -> impl IntoResponse {
        let reader = state.reader.read().await;
        
        match reader.lookup(&ip) {
            Ok(info) => {
                let response = IpResponse {
                    ip: info.ip,
                    ip_range: info.ip_range,
                    country: info.country,
                    city: info.city,
                    asn: info.asn,
                    organization: info.organization,
                };
                
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
} 