use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A Stellar network that SorobanLens can query.
///
/// Which networks are actually usable depends on the RPC endpoints configured
/// on the backend; see `GET /api/v1` for the list of enabled networks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Network {
    /// The public Stellar network (also called "pubnet" or "public").
    #[serde(alias = "public", alias = "pubnet")]
    Mainnet,
    Testnet,
    Futurenet,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown network `{0}` (expected one of: mainnet, testnet, futurenet)")]
pub struct UnknownNetwork(pub String);

impl Network {
    pub const ALL: [Network; 3] = [Network::Mainnet, Network::Testnet, Network::Futurenet];

    pub fn as_str(self) -> &'static str {
        match self {
            Network::Mainnet => "mainnet",
            Network::Testnet => "testnet",
            Network::Futurenet => "futurenet",
        }
    }
}

impl fmt::Display for Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Network {
    type Err = UnknownNetwork;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "mainnet" | "public" | "pubnet" => Ok(Network::Mainnet),
            "testnet" => Ok(Network::Testnet),
            "futurenet" => Ok(Network::Futurenet),
            other => Err(UnknownNetwork(other.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_names_and_aliases() {
        assert_eq!("testnet".parse::<Network>(), Ok(Network::Testnet));
        assert_eq!(" PUBNET ".parse::<Network>(), Ok(Network::Mainnet));
        assert_eq!("public".parse::<Network>(), Ok(Network::Mainnet));
        assert!("devnet".parse::<Network>().is_err());
    }

    #[test]
    fn serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&Network::Mainnet).expect("serialize"),
            "\"mainnet\""
        );
        let parsed: Network = serde_json::from_str("\"pubnet\"").expect("alias");
        assert_eq!(parsed, Network::Mainnet);
    }
}
