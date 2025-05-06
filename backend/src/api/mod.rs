mod ip_api;

use axum::Router;
use tower_http::cors::{Any, CorsLayer};

pub use ip_api::IpApiHandler;

pub fn create_router(ip_handler: IpApiHandler) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .merge(ip_handler.router())
        .layer(cors)
} 