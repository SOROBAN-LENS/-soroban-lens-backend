use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use soroban_lens_models::{Network, TransactionHash, TransactionInformation};

use crate::{RpcError, StellarRpcClient, TransactionSource};

/// Routes requests to the RPC client configured for each network.
#[derive(Debug, Default, Clone)]
pub struct NetworkRouter {
    clients: BTreeMap<Network, Arc<StellarRpcClient>>,
}

impl NetworkRouter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a client. Replaces any client already registered for the
    /// same network.
    pub fn with_client(mut self, client: StellarRpcClient) -> Self {
        self.clients.insert(client.network(), Arc::new(client));
        self
    }

    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }
}

#[async_trait]
impl TransactionSource for NetworkRouter {
    async fn get_transaction(
        &self,
        network: Network,
        hash: &TransactionHash,
    ) -> Result<TransactionInformation, RpcError> {
        let client = self
            .clients
            .get(&network)
            .ok_or(RpcError::NetworkNotConfigured(network))?;
        client.get_transaction(hash).await
    }

    fn networks(&self) -> Vec<Network> {
        self.clients.keys().copied().collect()
    }
}
