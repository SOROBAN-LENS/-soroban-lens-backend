//! SorobanLens REST API.
//!
//! The router is built by [`build_router`] from an [`AppState`], so tests can
//! construct the full HTTP stack with a mocked transaction source.

pub mod config;
pub mod error;
pub mod rate_limit;
pub mod routes;
pub mod store;

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderName, HeaderValue, Method, StatusCode, header};
use axum::routing::{get, post};
use axum::{Router, middleware};
use chrono::{DateTime, Utc};
use soroban_lens_diagnostic_engine::DiagnosticEngine;
use soroban_lens_models::Network;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::config::{Config, CorsOrigins};
use crate::rate_limit::RateLimiter;
use crate::store::AnalysisStore;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<DiagnosticEngine>,
    pub store: Arc<dyn AnalysisStore>,
    pub default_network: Network,
    pub started_at: DateTime<Utc>,
}

/// Builds the complete router, including middleware.
pub fn build_router(state: AppState, config: &Config) -> Router {
    let limiter = Arc::new(RateLimiter::new(&config.rate_limit));

    // Only the expensive endpoint (it calls Stellar RPC) is rate-limited.
    let analyze_post = post(routes::analyze).route_layer(middleware::from_fn_with_state(
        limiter,
        rate_limit::middleware,
    ));

    let api = Router::new()
        .route("/", get(routes::api_info))
        .route("/analyze", analyze_post)
        .route("/analyze/{id}", get(routes::get_analysis))
        .route("/stats", get(routes::stats));

    let request_id = HeaderName::from_static("x-request-id");

    Router::new()
        .route("/health", get(routes::health))
        .nest("/api/v1", api)
        .fallback(routes::not_found)
        .with_state(state)
        .layer(DefaultBodyLimit::max(config.max_body_bytes))
        .layer(cors_layer(&config.cors_origins))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            config.request_timeout,
        ))
        .layer(PropagateRequestIdLayer::new(request_id.clone()))
        .layer(TraceLayer::new_for_http())
        .layer(SetRequestIdLayer::new(request_id, MakeRequestUuid))
}

fn cors_layer(origins: &CorsOrigins) -> CorsLayer {
    let base = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::CONTENT_TYPE])
        .expose_headers([header::LOCATION, HeaderName::from_static("x-request-id")]);
    match origins {
        CorsOrigins::None => base,
        CorsOrigins::Any => base.allow_origin(AllowOrigin::any()),
        CorsOrigins::List(list) => {
            let values: Vec<HeaderValue> = list
                .iter()
                .filter_map(|o| HeaderValue::from_str(o).ok())
                .collect();
            base.allow_origin(values)
        }
    }
}
