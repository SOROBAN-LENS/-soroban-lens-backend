//! Analyze a live transaction from the command line.
//!
//! ```sh
//! cargo run -p soroban-lens-diagnostic-engine --example analyze -- <tx-hash> [rpc-url]
//! ```
//!
//! Defaults to the SDF testnet RPC endpoint. Prints the report as JSON.

use std::sync::Arc;

use soroban_lens_diagnostic_engine::DiagnosticEngine;
use soroban_lens_models::{Network, TransactionHash};
use soroban_lens_stellar_client::{RpcClientConfig, StellarRpcClient};
use url::Url;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let hash = TransactionHash::parse(&args.next().ok_or("usage: analyze <tx-hash> [rpc-url]")?)?;
    let url = Url::parse(
        &args
            .next()
            .unwrap_or_else(|| "https://soroban-testnet.stellar.org".to_owned()),
    )?;

    let client = StellarRpcClient::new(RpcClientConfig::new(Network::Testnet, url))?;
    let engine = DiagnosticEngine::new(Arc::new(client));
    let report = engine.analyze(Network::Testnet, &hash).await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
