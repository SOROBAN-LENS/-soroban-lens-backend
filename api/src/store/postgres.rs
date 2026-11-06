//! PostgreSQL-backed [`AnalysisStore`].
//!
//! Each report is stored as a JSONB document. Fields used for lookups and
//! aggregation are duplicated into indexed columns (see `api/migrations/`).
//! Queries are built at runtime (not with `sqlx::query!`) so the crate builds
//! without a live database or an offline query cache.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use soroban_lens_models::DiagnosticReport;
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::types::Json;
use uuid::Uuid;

use super::{AnalysisStore, StoreError, StoreStats};

/// Embedded schema migrations, applied by [`PostgresStore::migrate`].
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Connection settings for [`PostgresStore::connect`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostgresOptions {
    pub max_connections: u32,
    /// How long to wait for a free pooled connection before failing a request.
    pub acquire_timeout: Duration,
}

impl Default for PostgresOptions {
    fn default() -> Self {
        Self {
            max_connections: 10,
            acquire_timeout: Duration::from_secs(5),
        }
    }
}

pub struct PostgresStore {
    pool: PgPool,
}

impl PostgresStore {
    /// Connects to `url`, verifying the connection immediately.
    ///
    /// Errors never include the URL, which may contain credentials.
    pub async fn connect(url: &str, options: &PostgresOptions) -> Result<Self, StoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(options.max_connections)
            .acquire_timeout(options.acquire_timeout)
            .connect(url)
            .await
            .map_err(|e| StoreError::Unavailable(describe(&e)))?;
        Ok(Self { pool })
    }

    /// Wraps an existing pool (used by tests to control the schema).
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Applies pending schema migrations.
    pub async fn migrate(&self) -> Result<(), StoreError> {
        MIGRATOR
            .run(&self.pool)
            .await
            .map_err(|e| StoreError::Unavailable(format!("migration failed: {e}")))
    }
}

#[async_trait]
impl AnalysisStore for PostgresStore {
    async fn save(&self, report: DiagnosticReport) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO analyses \
                 (id, transaction_hash, network, transaction_status, analysis_status, \
                  schema_version, created_at, report) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (id) DO UPDATE SET \
                 transaction_hash = EXCLUDED.transaction_hash, \
                 network = EXCLUDED.network, \
                 transaction_status = EXCLUDED.transaction_status, \
                 analysis_status = EXCLUDED.analysis_status, \
                 schema_version = EXCLUDED.schema_version, \
                 created_at = EXCLUDED.created_at, \
                 report = EXCLUDED.report",
        )
        .bind(report.id)
        .bind(report.transaction_hash.to_string())
        .bind(label(&report.network)?)
        .bind(label(&report.transaction_status)?)
        .bind(label(&report.analysis_status)?)
        .bind(&report.metadata.schema_version)
        .bind(report.metadata.analyzed_at)
        .bind(Json(&report))
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(())
    }

    async fn get(&self, id: Uuid) -> Result<Option<DiagnosticReport>, StoreError> {
        let row: Option<(Json<DiagnosticReport>,)> =
            sqlx::query_as("SELECT report FROM analyses WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(db_error)?;
        Ok(row.map(|(Json(report),)| report))
    }

    async fn recent(&self, limit: usize) -> Result<Vec<DiagnosticReport>, StoreError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows: Vec<(Json<DiagnosticReport>,)> = sqlx::query_as(
            "SELECT report FROM analyses ORDER BY created_at DESC, id DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(rows.into_iter().map(|(Json(report),)| report).collect())
    }

    async fn stats(&self) -> Result<StoreStats, StoreError> {
        let (total, success, failed, not_found, since): (
            i64,
            i64,
            i64,
            i64,
            Option<DateTime<Utc>>,
        ) = sqlx::query_as(
            "SELECT \
                 COUNT(*), \
                 COUNT(*) FILTER (WHERE transaction_status = 'success'), \
                 COUNT(*) FILTER (WHERE transaction_status = 'failed'), \
                 COUNT(*) FILTER (WHERE transaction_status = 'not_found'), \
                 MIN(created_at) \
             FROM analyses",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(db_error)?;
        let count = |n: i64| u64::try_from(n).unwrap_or(0);
        Ok(StoreStats {
            since,
            total: count(total),
            success: count(success),
            failed: count(failed),
            not_found: count(not_found),
        })
    }

    fn kind(&self) -> &'static str {
        "postgres"
    }
}

/// The serde name of a unit enum variant (e.g. `TransactionStatus::NotFound`
/// → `"not_found"`), so columns match the JSON the API returns.
fn label<T: Serialize>(value: &T) -> Result<String, StoreError> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => Ok(s),
        _ => Err(StoreError::Unavailable(
            "failed to encode enum column".into(),
        )),
    }
}

fn db_error(e: sqlx::Error) -> StoreError {
    StoreError::Unavailable(describe(&e))
}

/// A log-safe description of a database error. Connection-level errors are
/// reduced to their kind because their messages can echo connection details.
fn describe(e: &sqlx::Error) -> String {
    match e {
        sqlx::Error::Configuration(_) => "invalid database configuration".into(),
        sqlx::Error::Io(_) | sqlx::Error::Tls(_) => "cannot reach database".into(),
        sqlx::Error::PoolTimedOut => "timed out waiting for a database connection".into(),
        sqlx::Error::PoolClosed => "database pool is closed".into(),
        sqlx::Error::Database(db) => format!("database error: {}", db.message()),
        sqlx::Error::Decode(_) | sqlx::Error::ColumnDecode { .. } => {
            "failed to decode stored analysis".into()
        }
        other => format!("database error: {other}"),
    }
}
