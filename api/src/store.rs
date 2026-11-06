//! Storage for completed analyses.
//!
//! The [`AnalysisStore`] trait is the persistence boundary. This milestone
//! ships a bounded in-memory implementation; reports are lost on restart.
//!
//! TODO(next milestone): add a PostgreSQL implementation (sqlx) storing the
//! report as JSONB alongside indexed columns (id, transaction_hash, network,
//! transaction_status, created_at), selected via a `DATABASE_URL` setting.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use async_trait::async_trait;
use soroban_lens_models::{DiagnosticReport, TransactionStatus};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("analysis store is unavailable: {0}")]
    Unavailable(String),
}

/// Aggregate counts over stored analyses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreStats {
    pub total: u64,
    pub success: u64,
    pub failed: u64,
    pub not_found: u64,
}

#[async_trait]
pub trait AnalysisStore: Send + Sync {
    async fn save(&self, report: DiagnosticReport) -> Result<(), StoreError>;
    async fn get(&self, id: Uuid) -> Result<Option<DiagnosticReport>, StoreError>;
    /// Most recent reports, newest first.
    async fn recent(&self, limit: usize) -> Result<Vec<DiagnosticReport>, StoreError>;
    /// Counts of every analysis saved since the store was created (including
    /// reports that have since been evicted).
    async fn stats(&self) -> Result<StoreStats, StoreError>;
    /// Short label describing the backend, e.g. `"in_memory"`.
    fn kind(&self) -> &'static str;
}

/// Bounded in-memory store. When full, the oldest report is evicted.
pub struct InMemoryStore {
    capacity: usize,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    reports: HashMap<Uuid, DiagnosticReport>,
    order: VecDeque<Uuid>,
    stats: StoreStats,
}

impl InMemoryStore {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            inner: Mutex::new(Inner::default()),
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Inner>, StoreError> {
        self.inner
            .lock()
            .map_err(|_| StoreError::Unavailable("store lock poisoned".into()))
    }
}

#[async_trait]
impl AnalysisStore for InMemoryStore {
    async fn save(&self, report: DiagnosticReport) -> Result<(), StoreError> {
        let mut inner = self.lock()?;
        inner.stats.total += 1;
        match report.transaction_status {
            TransactionStatus::Success => inner.stats.success += 1,
            TransactionStatus::Failed => inner.stats.failed += 1,
            TransactionStatus::NotFound => inner.stats.not_found += 1,
        }
        if inner.reports.insert(report.id, report.clone()).is_none() {
            inner.order.push_back(report.id);
        }
        while inner.order.len() > self.capacity {
            if let Some(oldest) = inner.order.pop_front() {
                inner.reports.remove(&oldest);
            }
        }
        Ok(())
    }

    async fn get(&self, id: Uuid) -> Result<Option<DiagnosticReport>, StoreError> {
        Ok(self.lock()?.reports.get(&id).cloned())
    }

    async fn recent(&self, limit: usize) -> Result<Vec<DiagnosticReport>, StoreError> {
        let inner = self.lock()?;
        Ok(inner
            .order
            .iter()
            .rev()
            .take(limit)
            .filter_map(|id| inner.reports.get(id).cloned())
            .collect())
    }

    async fn stats(&self) -> Result<StoreStats, StoreError> {
        Ok(self.lock()?.stats.clone())
    }

    fn kind(&self) -> &'static str {
        "in_memory"
    }
}
