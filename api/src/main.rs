use std::net::SocketAddr;
use std::sync::Arc;

use chrono::Utc;
use soroban_lens_api::config::{Config, LogFormat};
use soroban_lens_api::store::postgres::{PostgresOptions, PostgresStore};
use soroban_lens_api::store::{AnalysisStore, InMemoryStore};
use soroban_lens_api::{AppState, build_router};
use soroban_lens_diagnostic_engine::DiagnosticEngine;
use soroban_lens_stellar_client::{NetworkRouter, RpcClientConfig, StellarRpcClient};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    // A missing .env file is fine; real deployments use the environment.
    let _ = dotenvy::dotenv();

    let config = match Config::from_env() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("configuration error: {e}");
            std::process::exit(2);
        }
    };
    init_tracing(config.log_format);

    if let Err(e) = run(config).await {
        tracing::error!(error = %e, "server error");
        std::process::exit(1);
    }
}

async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let mut router = NetworkRouter::new();
    for (network, url) in &config.rpc_endpoints {
        let mut rpc = RpcClientConfig::new(*network, url.clone());
        rpc.request_timeout = config.rpc_timeout;
        let client = StellarRpcClient::new(rpc)?;
        tracing::info!(%network, endpoint = client.endpoint_label(), "configured Stellar RPC endpoint");
        router = router.with_client(client);
    }

    let state = AppState {
        engine: Arc::new(DiagnosticEngine::new(Arc::new(router))),
        store: build_store(&config).await?,
        default_network: config.default_network,
        started_at: Utc::now(),
    };
    let app = build_router(state, &config);

    let addr = SocketAddr::new(config.host, config.port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "SorobanLens API listening");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    tracing::info!("server stopped");
    Ok(())
}

async fn build_store(
    config: &Config,
) -> Result<Arc<dyn AnalysisStore>, Box<dyn std::error::Error>> {
    match &config.database {
        Some(db) => {
            let options = PostgresOptions {
                max_connections: db.max_connections,
                ..PostgresOptions::default()
            };
            let store = PostgresStore::connect(&db.url, &options).await?;
            store.migrate().await?;
            tracing::info!("using PostgreSQL analysis store (migrations applied)");
            Ok(Arc::new(store))
        }
        None => {
            tracing::warn!(
                capacity = config.store_capacity,
                "DATABASE_URL not set: using in-memory analysis store (lost on restart)"
            );
            Ok(Arc::new(InMemoryStore::new(config.store_capacity)))
        }
    }
}

fn init_tracing(format: LogFormat) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,soroban_lens=debug,tower_http=info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match format {
        LogFormat::Json => builder.json().init(),
        LogFormat::Pretty => builder.init(),
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received");
}
