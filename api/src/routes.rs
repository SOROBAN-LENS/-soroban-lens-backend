//! HTTP handlers. See `docs/API.md` for the full contract.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::Serialize;
use soroban_lens_diagnostic_engine::{ENGINE_VERSION, XDR_VERSION};
use soroban_lens_models::{
    AnalysisStatus, Network, REPORT_SCHEMA_VERSION, TransactionHash, TransactionRequest,
    TransactionStatus,
};
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiError;

const SERVICE_NAME: &str = "soroban-lens-api";
const API_VERSION: &str = "v1";
const RECENT_LIMIT: usize = 10;

#[derive(Serialize)]
pub struct HealthResponse {
    status: &'static str,
    service: &'static str,
    version: &'static str,
}

/// `GET /health` — liveness. Does not call upstream services.
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: SERVICE_NAME,
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[derive(Serialize)]
pub struct ApiInfo {
    name: &'static str,
    version: &'static str,
    api_version: &'static str,
    default_network: Network,
    networks: Vec<Network>,
    engine: EngineInfo,
    persistence: &'static str,
    endpoints: Vec<EndpointInfo>,
}

#[derive(Serialize)]
pub struct EngineInfo {
    version: &'static str,
    report_schema_version: &'static str,
    stellar_xdr_version: &'static str,
    rules: Vec<&'static str>,
}

#[derive(Serialize)]
pub struct EndpointInfo {
    method: &'static str,
    path: &'static str,
    description: &'static str,
}

/// `GET /api/v1` — API metadata and enabled networks.
pub async fn api_info(State(state): State<AppState>) -> Json<ApiInfo> {
    Json(ApiInfo {
        name: "SorobanLens API",
        version: env!("CARGO_PKG_VERSION"),
        api_version: API_VERSION,
        default_network: state.default_network,
        networks: state.engine.networks(),
        engine: EngineInfo {
            version: ENGINE_VERSION,
            report_schema_version: REPORT_SCHEMA_VERSION,
            stellar_xdr_version: XDR_VERSION,
            rules: state.engine.rule_names(),
        },
        persistence: state.store.kind(),
        endpoints: vec![
            EndpointInfo {
                method: "GET",
                path: "/health",
                description: "Liveness check",
            },
            EndpointInfo {
                method: "GET",
                path: "/api/v1",
                description: "API information",
            },
            EndpointInfo {
                method: "POST",
                path: "/api/v1/analyze",
                description: "Analyze a transaction by hash",
            },
            EndpointInfo {
                method: "GET",
                path: "/api/v1/analyze/{id}",
                description: "Retrieve a stored analysis",
            },
            EndpointInfo {
                method: "GET",
                path: "/api/v1/stats",
                description: "Analysis counts and recent activity for this server",
            },
        ],
    })
}

/// `POST /api/v1/analyze` — fetch and analyze a transaction.
///
/// Returns `201 Created` with the report and a `Location` header.
pub async fn analyze(
    State(state): State<AppState>,
    payload: Result<Json<TransactionRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(request) = payload?;
    let hash = request
        .validated_hash()
        .map_err(|e| ApiError::bad_request("invalid_transaction_hash", e.to_string()))?;
    let network = request.network.unwrap_or(state.default_network);

    if !state.engine.networks().contains(&network) {
        return Err(ApiError::bad_request(
            "network_not_configured",
            format!("Network `{network}` is not enabled on this server."),
        ));
    }

    tracing::info!(%hash, %network, "analyzing transaction");
    let report = state.engine.analyze(network, &hash).await?;
    tracing::info!(
        %hash,
        id = %report.id,
        transaction_status = ?report.transaction_status,
        issues = report.issues.len(),
        "analysis complete"
    );

    state.store.save(report.clone()).await.map_err(|e| {
        tracing::error!(error = %e, "failed to store analysis");
        ApiError::internal()
    })?;

    let location = format!("/api/v1/analyze/{}", report.id);
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, location)],
        Json(report),
    )
        .into_response())
}

/// `GET /api/v1/analyze/{id}` — retrieve a stored report.
pub async fn get_analysis(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let id = Uuid::parse_str(&id)
        .map_err(|_| ApiError::bad_request("invalid_analysis_id", "Analysis ID must be a UUID."))?;
    let report = state.store.get(id).await.map_err(|e| {
        tracing::error!(error = %e, "failed to read analysis");
        ApiError::internal()
    })?;
    match report {
        Some(report) => Ok(Json(report).into_response()),
        None => Err(ApiError::not_found(
            "analysis_not_found",
            "No analysis exists with this ID. Analyses are kept in memory and are lost when the server restarts.",
        )),
    }
}

#[derive(Serialize)]
pub struct StatsResponse {
    /// Counts since this server process started.
    since: DateTime<Utc>,
    persistence: &'static str,
    total_analyses: u64,
    successful_transactions: u64,
    failed_transactions: u64,
    not_found_transactions: u64,
    recent: Vec<RecentAnalysis>,
}

#[derive(Serialize)]
pub struct RecentAnalysis {
    id: Uuid,
    transaction_hash: TransactionHash,
    network: Network,
    transaction_status: TransactionStatus,
    analysis_status: AnalysisStatus,
    summary: String,
    issue_count: usize,
    analyzed_at: DateTime<Utc>,
}

/// `GET /api/v1/stats` — real counts of analyses served by this instance.
pub async fn stats(State(state): State<AppState>) -> Result<Json<StatsResponse>, ApiError> {
    let internal = |e: crate::store::StoreError| {
        tracing::error!(error = %e, "failed to read stats");
        ApiError::internal()
    };
    let counts = state.store.stats().await.map_err(internal)?;
    let recent = state
        .store
        .recent(RECENT_LIMIT)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|r| RecentAnalysis {
            id: r.id,
            transaction_hash: r.transaction_hash,
            network: r.network,
            transaction_status: r.transaction_status,
            analysis_status: r.analysis_status,
            issue_count: r.issues.len(),
            summary: r.summary,
            analyzed_at: r.metadata.analyzed_at,
        })
        .collect();
    Ok(Json(StatsResponse {
        since: counts.since.unwrap_or(state.started_at),
        persistence: state.store.kind(),
        total_analyses: counts.total,
        successful_transactions: counts.success,
        failed_transactions: counts.failed,
        not_found_transactions: counts.not_found,
        recent,
    }))
}

pub async fn not_found() -> ApiError {
    ApiError::not_found("route_not_found", "No such endpoint.")
}
