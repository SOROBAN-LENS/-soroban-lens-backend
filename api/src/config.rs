//! Runtime configuration, loaded from environment variables.
//!
//! See `.env.example` for every supported variable. No secrets have defaults;
//! RPC URLs that embed provider API keys must be supplied via the environment.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::Duration;

use soroban_lens_models::Network;
use url::Url;

/// Default public testnet RPC endpoint operated by the Stellar Development
/// Foundation. Suitable for development; not for production traffic.
pub const DEFAULT_TESTNET_RPC_URL: &str = "https://soroban-testnet.stellar.org";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Pretty,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorsOrigins {
    /// No cross-origin access.
    None,
    /// Allow any origin (development only).
    Any,
    List(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitConfig {
    /// Maximum analysis requests per client per window. `0` disables limiting.
    pub max_requests: u32,
    pub window: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub host: IpAddr,
    pub port: u16,
    pub default_network: Network,
    pub rpc_endpoints: BTreeMap<Network, Url>,
    pub rpc_timeout: Duration,
    pub request_timeout: Duration,
    pub cors_origins: CorsOrigins,
    pub max_body_bytes: usize,
    pub rate_limit: RateLimitConfig,
    pub store_capacity: usize,
    pub log_format: LogFormat,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid value for {var}: {reason}")]
    Invalid { var: &'static str, reason: String },
    #[error("no Stellar RPC endpoint is configured; set at least STELLAR_RPC_TESTNET_URL")]
    NoEndpoints,
    #[error("default network `{0}` has no RPC endpoint configured")]
    DefaultNetworkNotConfigured(Network),
}

impl Config {
    /// Loads configuration from the process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Loads configuration using an arbitrary lookup function (used in tests).
    pub fn from_lookup<F>(lookup: F) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let get = |key: &str| lookup(key).map(|v| v.trim().to_owned());

        let host = parse_or(
            &get,
            "SOROBAN_LENS_HOST",
            "127.0.0.1".parse().expect("valid ip"),
        )?;
        let port = parse_or(&get, "SOROBAN_LENS_PORT", 8080u16)?;
        let default_network = parse_or(&get, "SOROBAN_LENS_DEFAULT_NETWORK", Network::Testnet)?;

        let mut rpc_endpoints = BTreeMap::new();
        for (network, var, default) in [
            (
                Network::Testnet,
                "STELLAR_RPC_TESTNET_URL",
                Some(DEFAULT_TESTNET_RPC_URL),
            ),
            (Network::Mainnet, "STELLAR_RPC_MAINNET_URL", None),
            (Network::Futurenet, "STELLAR_RPC_FUTURENET_URL", None),
        ] {
            // An explicitly empty variable disables the network.
            let value = match get(var) {
                Some(v) if v.is_empty() => None,
                Some(v) => Some(v),
                None => default.map(str::to_owned),
            };
            if let Some(raw) = value {
                let url = Url::parse(&raw).map_err(|e| ConfigError::Invalid {
                    var,
                    reason: e.to_string(),
                })?;
                if !matches!(url.scheme(), "http" | "https") {
                    return Err(ConfigError::Invalid {
                        var,
                        reason: "URL scheme must be http or https".into(),
                    });
                }
                rpc_endpoints.insert(network, url);
            }
        }
        if rpc_endpoints.is_empty() {
            return Err(ConfigError::NoEndpoints);
        }
        if !rpc_endpoints.contains_key(&default_network) {
            return Err(ConfigError::DefaultNetworkNotConfigured(default_network));
        }

        let rpc_timeout = Duration::from_secs(positive(&get, "STELLAR_RPC_TIMEOUT_SECS", 10)?);
        let request_timeout =
            Duration::from_secs(positive(&get, "SOROBAN_LENS_REQUEST_TIMEOUT_SECS", 30)?);

        let cors_origins = match get("SOROBAN_LENS_CORS_ORIGINS") {
            None => CorsOrigins::List(vec!["http://localhost:5173".into()]),
            Some(v) if v.is_empty() => CorsOrigins::None,
            Some(v) if v == "*" => CorsOrigins::Any,
            Some(v) => {
                let origins: Vec<String> = v
                    .split(',')
                    .map(|s| s.trim().trim_end_matches('/').to_owned())
                    .filter(|s| !s.is_empty())
                    .collect();
                for origin in &origins {
                    let parsed = Url::parse(origin).map_err(|e| ConfigError::Invalid {
                        var: "SOROBAN_LENS_CORS_ORIGINS",
                        reason: format!("`{origin}`: {e}"),
                    })?;
                    if parsed.path() != "/" || parsed.query().is_some() {
                        return Err(ConfigError::Invalid {
                            var: "SOROBAN_LENS_CORS_ORIGINS",
                            reason: format!("`{origin}` must be an origin (scheme://host[:port])"),
                        });
                    }
                }
                CorsOrigins::List(origins)
            }
        };

        let max_body_bytes = positive(&get, "SOROBAN_LENS_MAX_BODY_BYTES", 16 * 1024)? as usize;
        let rate_limit = RateLimitConfig {
            max_requests: parse_or(&get, "SOROBAN_LENS_RATE_LIMIT_PER_MINUTE", 30u32)?,
            window: Duration::from_secs(60),
        };
        let store_capacity = positive(&get, "SOROBAN_LENS_STORE_CAPACITY", 1000)? as usize;
        let log_format = match get("SOROBAN_LENS_LOG_FORMAT").as_deref() {
            None | Some("pretty") => LogFormat::Pretty,
            Some("json") => LogFormat::Json,
            Some(other) => {
                return Err(ConfigError::Invalid {
                    var: "SOROBAN_LENS_LOG_FORMAT",
                    reason: format!("expected `pretty` or `json`, got `{other}`"),
                });
            }
        };

        Ok(Self {
            host,
            port,
            default_network,
            rpc_endpoints,
            rpc_timeout,
            request_timeout,
            cors_origins,
            max_body_bytes,
            rate_limit,
            store_capacity,
            log_format,
        })
    }
}

fn parse_or<T, G>(get: &G, var: &'static str, default: T) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
    G: Fn(&str) -> Option<String>,
{
    match get(var) {
        None => Ok(default),
        Some(v) if v.is_empty() => Ok(default),
        Some(v) => v.parse().map_err(|e: T::Err| ConfigError::Invalid {
            var,
            reason: e.to_string(),
        }),
    }
}

fn positive<G>(get: &G, var: &'static str, default: u64) -> Result<u64, ConfigError>
where
    G: Fn(&str) -> Option<String>,
{
    let value = parse_or(get, var, default)?;
    if value == 0 {
        return Err(ConfigError::Invalid {
            var,
            reason: "must be greater than zero".into(),
        });
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        Config::from_lookup(|k| map.get(k).cloned())
    }

    #[test]
    fn defaults_are_safe_for_local_development() {
        let c = config(&[]).expect("defaults");
        assert_eq!(c.host.to_string(), "127.0.0.1");
        assert_eq!(c.port, 8080);
        assert_eq!(c.default_network, Network::Testnet);
        assert_eq!(
            c.rpc_endpoints[&Network::Testnet].as_str(),
            "https://soroban-testnet.stellar.org/"
        );
        assert!(!c.rpc_endpoints.contains_key(&Network::Mainnet));
        assert_eq!(
            c.cors_origins,
            CorsOrigins::List(vec!["http://localhost:5173".into()])
        );
        assert_eq!(c.rate_limit.max_requests, 30);
    }

    #[test]
    fn parses_custom_values() {
        let c = config(&[
            ("SOROBAN_LENS_PORT", "9000"),
            ("STELLAR_RPC_MAINNET_URL", "https://rpc.example.com/key"),
            ("SOROBAN_LENS_DEFAULT_NETWORK", "mainnet"),
            (
                "SOROBAN_LENS_CORS_ORIGINS",
                "https://a.example, https://b.example/",
            ),
            ("SOROBAN_LENS_LOG_FORMAT", "json"),
        ])
        .expect("config");
        assert_eq!(c.port, 9000);
        assert_eq!(c.default_network, Network::Mainnet);
        assert_eq!(
            c.cors_origins,
            CorsOrigins::List(vec!["https://a.example".into(), "https://b.example".into()])
        );
        assert_eq!(c.log_format, LogFormat::Json);
    }

    #[test]
    fn rejects_default_network_without_endpoint() {
        assert_eq!(
            config(&[("SOROBAN_LENS_DEFAULT_NETWORK", "mainnet")]),
            Err(ConfigError::DefaultNetworkNotConfigured(Network::Mainnet))
        );
    }

    #[test]
    fn empty_testnet_url_disables_testnet() {
        assert_eq!(
            config(&[("STELLAR_RPC_TESTNET_URL", "")]),
            Err(ConfigError::NoEndpoints)
        );
    }

    #[test]
    fn rejects_invalid_values() {
        assert!(config(&[("SOROBAN_LENS_PORT", "abc")]).is_err());
        assert!(config(&[("STELLAR_RPC_TESTNET_URL", "ftp://x.example")]).is_err());
        assert!(config(&[("STELLAR_RPC_TIMEOUT_SECS", "0")]).is_err());
        assert!(config(&[("SOROBAN_LENS_CORS_ORIGINS", "https://a.example/path")]).is_err());
        assert!(config(&[("SOROBAN_LENS_LOG_FORMAT", "xml")]).is_err());
    }

    #[test]
    fn cors_wildcard_and_disable() {
        assert_eq!(
            config(&[("SOROBAN_LENS_CORS_ORIGINS", "*")])
                .expect("ok")
                .cors_origins,
            CorsOrigins::Any
        );
        assert_eq!(
            config(&[("SOROBAN_LENS_CORS_ORIGINS", "")])
                .expect("ok")
                .cors_origins,
            CorsOrigins::None
        );
    }
}
