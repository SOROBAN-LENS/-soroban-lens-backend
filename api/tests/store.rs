//! Behaviour every `AnalysisStore` must share, run against each implementation.
//!
//! PostgreSQL tests need a database and run only when `TEST_DATABASE_URL` is
//! set (CI sets it); otherwise they print a notice and pass. Each test creates
//! and drops its own schema, so tests can run in parallel against one database.
//!
//! ```sh
//! docker run -d --name soroban-lens-pg -p 5432:5432 \
//!   -e POSTGRES_USER=soroban_lens -e POSTGRES_PASSWORD=soroban_lens postgres:17-alpine
//! TEST_DATABASE_URL=postgres://soroban_lens:soroban_lens@localhost:5432/soroban_lens \
//!   cargo test -p soroban-lens-api --test store
//! ```

use std::collections::BTreeMap;
use std::str::FromStr;

use chrono::{DateTime, Duration, TimeZone, Utc};
use soroban_lens_api::store::postgres::PostgresStore;
use soroban_lens_api::store::{AnalysisStore, InMemoryStore};
use soroban_lens_models::{
    AnalysisStatus, DiagnosticEventAvailability, DiagnosticIssue, DiagnosticReport, ErrorCategory,
    Evidence, EvidenceLevel, Limitation, Network, OperationOverview, ReportMetadata, Severity,
    SuggestedFix, TransactionHash, TransactionOverview, TransactionStatus,
};
use sqlx::Executor;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use uuid::Uuid;

const HASH: &str = "ae853c97eb8a90190a1754061bc20a1d680e04b9c6b607f8a7369d7268c0244d";

fn base_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 6, 12, 0, 0)
        .single()
        .expect("valid time")
}

/// A report with every optional section filled, so round-trips are meaningful.
fn report(status: TransactionStatus, offset_secs: i64) -> DiagnosticReport {
    let analysis_status = match status {
        TransactionStatus::NotFound => AnalysisStatus::TransactionNotFound,
        _ => AnalysisStatus::Completed,
    };
    let mut details = BTreeMap::new();
    details.insert("function_name".to_owned(), "transfer".to_owned());
    DiagnosticReport {
        id: Uuid::new_v4(),
        transaction_hash: TransactionHash::from_str(HASH).expect("valid hash"),
        network: Network::Testnet,
        analysis_status,
        transaction_status: status,
        summary: "Transaction failed: Archived ledger entry in footprint.".into(),
        transaction: Some(TransactionOverview {
            ledger: Some(123),
            created_at: Some(1_762_430_400),
            application_order: Some(2),
            fee_bump: Some(false),
            fee_charged: Some(100_000),
            result_code: Some("tx_failed".into()),
            operations: vec![OperationOverview {
                index: 0,
                operation_type: "invoke_host_function".into(),
                result_code: Some("invoke_host_function_entry_archived".into()),
                contract_id: Some("CABC".into()),
                function_name: Some("transfer".into()),
            }],
            declared_resources: None,
        }),
        issues: vec![DiagnosticIssue {
            rule_id: "invoke.entry_archived".into(),
            category: ErrorCategory::ArchivedState,
            severity: Severity::Error,
            title: "Archived ledger entry in footprint".into(),
            explanation: "A ledger entry must be restored.".into(),
            technical_explanation: "INVOKE_HOST_FUNCTION_ENTRY_ARCHIVED".into(),
            possible_causes: vec!["TTL expired".into()],
            suggested_fixes: vec![SuggestedFix {
                title: "Restore the entry".into(),
                description: "Submit a RestoreFootprint operation.".into(),
                reference_url: Some("https://developers.stellar.org/".into()),
            }],
            evidence: Evidence {
                level: EvidenceLevel::Confirmed,
                sources: vec!["operation_result[0]".into()],
            },
            details,
        }],
        limitations: vec![Limitation {
            code: "example".into(),
            message: "Example limitation.".into(),
        }],
        metadata: ReportMetadata {
            schema_version: "1".into(),
            engine_version: "0.1.0".into(),
            analyzed_at: base_time() + Duration::seconds(offset_secs),
            latest_ledger: Some(200),
            oldest_ledger: Some(100),
            diagnostic_events: DiagnosticEventAvailability {
                available: true,
                count: 1,
            },
        },
    }
}

async fn contract_round_trip(store: &dyn AnalysisStore) {
    let saved = report(TransactionStatus::Failed, 0);
    store.save(saved.clone()).await.expect("save");
    assert_eq!(store.get(saved.id).await.expect("get"), Some(saved));
    assert_eq!(store.get(Uuid::new_v4()).await.expect("get missing"), None);
}

async fn contract_save_same_id_replaces(store: &dyn AnalysisStore) {
    let mut r = report(TransactionStatus::Failed, 0);
    store.save(r.clone()).await.expect("save");
    r.summary = "updated".into();
    store.save(r.clone()).await.expect("save again");
    assert_eq!(store.get(r.id).await.expect("get"), Some(r.clone()));
    let recent = store.recent(10).await.expect("recent");
    assert_eq!(recent.iter().filter(|x| x.id == r.id).count(), 1);
}

async fn contract_recent_is_newest_first_and_limited(store: &dyn AnalysisStore) {
    let reports: Vec<_> = (0..3)
        .map(|i| report(TransactionStatus::Failed, i))
        .collect();
    for r in &reports {
        store.save(r.clone()).await.expect("save");
    }
    let ids: Vec<Uuid> = store
        .recent(2)
        .await
        .expect("recent")
        .iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, vec![reports[2].id, reports[1].id]);
}

async fn contract_stats_count_by_status(store: &dyn AnalysisStore) {
    for (status, offset) in [
        (TransactionStatus::Failed, 0),
        (TransactionStatus::Failed, 1),
        (TransactionStatus::Success, 2),
        (TransactionStatus::NotFound, 3),
    ] {
        store.save(report(status, offset)).await.expect("save");
    }
    let stats = store.stats().await.expect("stats");
    assert_eq!(
        (stats.total, stats.failed, stats.success, stats.not_found),
        (4, 2, 1, 1)
    );
    assert!(stats.since.is_some());
}

// --- In-memory -----------------------------------------------------------------

#[tokio::test]
async fn in_memory_round_trip() {
    contract_round_trip(&InMemoryStore::new(10)).await;
}

#[tokio::test]
async fn in_memory_save_same_id_replaces() {
    contract_save_same_id_replaces(&InMemoryStore::new(10)).await;
}

#[tokio::test]
async fn in_memory_recent_is_newest_first_and_limited() {
    contract_recent_is_newest_first_and_limited(&InMemoryStore::new(10)).await;
}

#[tokio::test]
async fn in_memory_stats_count_by_status() {
    contract_stats_count_by_status(&InMemoryStore::new(10)).await;
}

#[tokio::test]
async fn in_memory_evicts_oldest_but_keeps_counting() {
    let store = InMemoryStore::new(2);
    let reports: Vec<_> = (0..3)
        .map(|i| report(TransactionStatus::Failed, i))
        .collect();
    for r in &reports {
        store.save(r.clone()).await.expect("save");
    }
    assert_eq!(store.get(reports[0].id).await.expect("get"), None);
    assert!(store.get(reports[2].id).await.expect("get").is_some());
    assert_eq!(store.stats().await.expect("stats").total, 3);
}

// --- PostgreSQL ------------------------------------------------------------------

/// A store using a fresh, uniquely named schema. Dropped by [`PgSchema::drop`].
struct PgSchema {
    store: PostgresStore,
    admin: sqlx::PgPool,
    name: String,
}

impl PgSchema {
    async fn create() -> Option<Self> {
        let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
            eprintln!("TEST_DATABASE_URL not set; skipping PostgreSQL store test");
            return None;
        };
        let name = format!("test_{}", Uuid::new_v4().simple());
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect to TEST_DATABASE_URL");
        admin
            .execute(format!("CREATE SCHEMA {name}").as_str())
            .await
            .expect("create schema");
        let options = PgConnectOptions::from_str(&url)
            .expect("parse TEST_DATABASE_URL")
            .options([("search_path", name.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .expect("connect with schema");
        let store = PostgresStore::from_pool(pool);
        store.migrate().await.expect("migrate");
        Some(Self { store, admin, name })
    }

    async fn drop(self) {
        self.admin
            .execute(format!("DROP SCHEMA {} CASCADE", self.name).as_str())
            .await
            .expect("drop schema");
    }
}

macro_rules! pg_contract_test {
    ($name:ident, $contract:ident) => {
        #[tokio::test]
        async fn $name() {
            let Some(schema) = PgSchema::create().await else {
                return;
            };
            $contract(&schema.store).await;
            schema.drop().await;
        }
    };
}

pg_contract_test!(postgres_round_trip, contract_round_trip);
pg_contract_test!(
    postgres_save_same_id_replaces,
    contract_save_same_id_replaces
);
pg_contract_test!(
    postgres_recent_is_newest_first_and_limited,
    contract_recent_is_newest_first_and_limited
);
pg_contract_test!(
    postgres_stats_count_by_status,
    contract_stats_count_by_status
);

#[tokio::test]
async fn postgres_migrations_are_idempotent_and_kind_is_reported() {
    let Some(schema) = PgSchema::create().await else {
        return;
    };
    schema
        .store
        .migrate()
        .await
        .expect("second migrate is a no-op");
    assert_eq!(schema.store.kind(), "postgres");
    let empty = schema.store.stats().await.expect("stats");
    assert_eq!((empty.total, empty.since), (0, None));
    schema.drop().await;
}

#[tokio::test]
async fn postgres_connect_error_does_not_leak_credentials() {
    use soroban_lens_api::store::postgres::PostgresOptions;
    let options = PostgresOptions {
        max_connections: 1,
        acquire_timeout: std::time::Duration::from_secs(2),
    };
    // Port 1 is reserved; nothing listens there.
    let result = PostgresStore::connect("postgres://user:hunter2@127.0.0.1:1/db", &options).await;
    let err = result.err().expect("connection must fail").to_string();
    assert!(!err.contains("hunter2"), "error leaked credentials: {err}");
}
