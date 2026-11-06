//! Per-client fixed-window rate limiting.
//!
//! This is a deliberately simple, single-instance limiter keyed by the TCP
//! peer address. It protects the upstream RPC node from accidental floods.
//!
//! Limitations / TODO(next milestone):
//! * Behind a reverse proxy every request shares the proxy's IP. Add an
//!   opt-in trusted-proxy setting that reads `X-Forwarded-For`.
//! * State is per process; multiple replicas need a shared store (e.g. Redis)
//!   or rate limiting at the gateway.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::config::RateLimitConfig;
use crate::error::ApiError;

/// Entries are pruned once the map grows past this many clients.
const PRUNE_THRESHOLD: usize = 10_000;

#[derive(Debug)]
pub struct RateLimiter {
    max_requests: u32,
    window: Duration,
    clients: Mutex<HashMap<IpAddr, Window>>,
}

#[derive(Debug, Clone, Copy)]
struct Window {
    started: Instant,
    count: u32,
}

impl RateLimiter {
    pub fn new(config: &RateLimitConfig) -> Self {
        Self {
            max_requests: config.max_requests,
            window: config.window,
            clients: Mutex::new(HashMap::new()),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.max_requests > 0
    }

    /// Records a request. Returns `Err(retry_after)` if the client is over
    /// its limit.
    pub fn check(&self, client: IpAddr, now: Instant) -> Result<(), Duration> {
        if !self.is_enabled() {
            return Ok(());
        }
        // A poisoned lock only means another request panicked mid-update; the
        // counters are still usable.
        let mut clients = self.clients.lock().unwrap_or_else(|p| p.into_inner());

        if clients.len() > PRUNE_THRESHOLD {
            let window = self.window;
            clients.retain(|_, w| now.duration_since(w.started) < window);
        }

        let entry = clients.entry(client).or_insert(Window {
            started: now,
            count: 0,
        });
        if now.duration_since(entry.started) >= self.window {
            *entry = Window {
                started: now,
                count: 0,
            };
        }
        if entry.count >= self.max_requests {
            let elapsed = now.duration_since(entry.started);
            return Err(self.window.saturating_sub(elapsed));
        }
        entry.count += 1;
        Ok(())
    }
}

/// Axum middleware applying the limiter.
pub async fn middleware(
    State(limiter): State<Arc<RateLimiter>>,
    request: Request,
    next: Next,
) -> Response {
    let client = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));

    match limiter.check(client, Instant::now()) {
        Ok(()) => next.run(request).await,
        Err(retry_after) => {
            tracing::info!(%client, "rate limit exceeded");
            let mut err = ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "Too many analysis requests. Please wait before trying again.",
            );
            err.retry_after = Some(retry_after.as_secs().max(1));
            err.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(max: u32) -> RateLimiter {
        RateLimiter::new(&RateLimitConfig {
            max_requests: max,
            window: Duration::from_secs(60),
        })
    }

    #[test]
    fn allows_up_to_limit_then_blocks() {
        let l = limiter(2);
        let ip: IpAddr = "10.0.0.1".parse().expect("ip");
        let now = Instant::now();
        assert!(l.check(ip, now).is_ok());
        assert!(l.check(ip, now).is_ok());
        let retry = l
            .check(ip, now + Duration::from_secs(10))
            .expect_err("blocked");
        assert_eq!(retry, Duration::from_secs(50));
    }

    #[test]
    fn window_resets() {
        let l = limiter(1);
        let ip: IpAddr = "10.0.0.1".parse().expect("ip");
        let now = Instant::now();
        assert!(l.check(ip, now).is_ok());
        assert!(l.check(ip, now).is_err());
        assert!(l.check(ip, now + Duration::from_secs(60)).is_ok());
    }

    #[test]
    fn clients_are_independent() {
        let l = limiter(1);
        let now = Instant::now();
        assert!(l.check("10.0.0.1".parse().expect("ip"), now).is_ok());
        assert!(l.check("10.0.0.2".parse().expect("ip"), now).is_ok());
    }

    #[test]
    fn zero_disables_limiting() {
        let l = limiter(0);
        let ip: IpAddr = "10.0.0.1".parse().expect("ip");
        for _ in 0..100 {
            assert!(l.check(ip, Instant::now()).is_ok());
        }
    }
}
