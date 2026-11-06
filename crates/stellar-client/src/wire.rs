//! JSON-RPC wire types, matching the Stellar RPC `getTransaction` reference:
//! <https://developers.stellar.org/docs/data/apis/rpc/api-reference/methods/getTransaction>

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub(crate) struct JsonRpcRequest<'a, P: Serialize> {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: &'a str,
    pub params: P,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JsonRpcResponse<T> {
    pub result: Option<T>,
    pub error: Option<JsonRpcErrorObject>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JsonRpcErrorObject {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct GetTransactionParams<'a> {
    pub hash: &'a str,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub(crate) enum WireTransactionStatus {
    #[serde(rename = "SUCCESS")]
    Success,
    #[serde(rename = "FAILED")]
    Failed,
    #[serde(rename = "NOT_FOUND")]
    NotFound,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GetTransactionResult {
    pub status: WireTransactionStatus,
    pub latest_ledger: u32,
    pub oldest_ledger: u32,
    pub ledger: Option<u32>,
    /// Unix timestamp. `getTransaction` encodes it as a string while
    /// `getTransactions` uses a number, so both are accepted.
    #[serde(default, deserialize_with = "string_or_number")]
    pub created_at: Option<String>,
    pub application_order: Option<u32>,
    pub fee_bump: Option<bool>,
    pub envelope_xdr: Option<String>,
    pub result_xdr: Option<String>,
    pub result_meta_xdr: Option<String>,
    /// Only present when the RPC operator has enabled diagnostic events.
    pub diagnostic_events_xdr: Option<Vec<String>>,
    // NOTE: RPC v23+ also returns an `events` object (transaction and contract
    // events). It is not needed for failure diagnosis yet and is ignored.
}

fn string_or_number<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Str(String),
        Num(i64),
    }
    Ok(
        Option::<Raw>::deserialize(deserializer)?.map(|raw| match raw {
            Raw::Str(s) => s,
            Raw::Num(n) => n.to_string(),
        }),
    )
}
