//! Built-in diagnostic rules.
//!
//! | Rule                    | Input signal                         | Rule IDs              |
//! |-------------------------|--------------------------------------|-----------------------|
//! | [`HostErrorRule`]       | `error` diagnostic events            | `host.*`              |
//! | [`OperationResultRule`] | operation result codes               | `invoke.*`, `op.*`, `extend_ttl.*`, `restore.*` |
//! | [`TransactionResultRule`] | transaction result codes           | `tx.*`                |
//! | [`NotFoundRule`]        | `NOT_FOUND` status from RPC          | `rpc.transaction_not_found` |
//!
//! TODO(next milestone): rules for simulation failures, classic operation
//! failures (payments, trustlines), and contract-specific error catalogs
//! (mapping `Error(Contract, #n)` to names from a contract's spec).

mod host_error;
mod not_found;
mod operation;
mod transaction;

pub use host_error::HostErrorRule;
pub use not_found::NotFoundRule;
pub use operation::OperationResultRule;
pub use transaction::TransactionResultRule;

use crate::Rule;

/// Rule priorities: lower values sort first among issues of equal severity
/// and evidence. More specific explanations get lower numbers.
pub(crate) mod priority {
    pub const HOST_ERROR: u8 = 10;
    pub const INVOKE: u8 = 20;
    pub const OPERATION: u8 = 30;
    pub const TRANSACTION: u8 = 40;
    pub const RPC: u8 = 50;
}

pub fn default_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(HostErrorRule),
        Box::new(OperationResultRule),
        Box::new(TransactionResultRule),
        Box::new(NotFoundRule),
    ]
}
