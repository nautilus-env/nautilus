//! Which address of a host name a pool connects to.
//!
//! sqlx tries the addresses a name resolves to one after the other. An address
//! that drops packets instead of refusing them holds each attempt for the
//! operating system's whole SYN timeout (about 21 s on Windows) before the next
//! one is tried, which is longer than the pool's acquire timeout: `localhost`
//! resolves to `::1` first, and Docker Desktop on Windows publishes ports on
//! IPv4 only, so a pool for `localhost` never opened.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::task::JoinSet;

/// How long an address may take to accept a TCP connection while the
/// addresses of a name are raced.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// The address of `host` that accepts a TCP connection first, when the name
/// resolves to more than one.
///
/// `None` keeps the host as written: an IP literal, a name with a single
/// address, or a name none of whose addresses answered, in which case the
/// regular connection reports the real error.
pub(crate) async fn first_reachable_address(host: &str, port: u16) -> Option<IpAddr> {
    if host.parse::<IpAddr>().is_ok() {
        return None;
    }
    let mut candidates: Vec<SocketAddr> =
        tokio::net::lookup_host((host, port)).await.ok()?.collect();
    candidates.dedup();
    if candidates.len() < 2 {
        return None;
    }
    first_accepting(&candidates).await.map(|addr| addr.ip())
}

/// The first of `candidates` to accept a TCP connection within
/// [`PROBE_TIMEOUT`], connecting to all of them at once.
async fn first_accepting(candidates: &[SocketAddr]) -> Option<SocketAddr> {
    let mut attempts = JoinSet::new();
    for &addr in candidates {
        attempts.spawn(async move {
            match tokio::time::timeout(PROBE_TIMEOUT, TcpStream::connect(addr)).await {
                Ok(Ok(_)) => Some(addr),
                _ => None,
            }
        });
    }
    while let Some(attempt) = attempts.join_next().await {
        if let Ok(Some(addr)) = attempt {
            return Some(addr);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn an_ip_literal_is_kept_as_written() {
        assert_eq!(first_reachable_address("127.0.0.1", 5432).await, None);
        assert_eq!(first_reachable_address("::1", 5432).await, None);
    }

    #[tokio::test]
    async fn the_answering_address_wins_over_one_that_drops_packets() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let open = listener.local_addr().unwrap();
        // TEST-NET-1 (RFC 5737) is never routed: the attempt hangs or fails.
        let silent: SocketAddr = format!("192.0.2.1:{}", open.port()).parse().unwrap();

        let started = Instant::now();
        let chosen = first_accepting(&[silent, open]).await;

        assert_eq!(chosen, Some(open));
        assert!(
            started.elapsed() < PROBE_TIMEOUT,
            "took {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn nothing_reachable_leaves_the_choice_to_the_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed = listener.local_addr().unwrap();
        drop(listener);

        assert_eq!(first_accepting(&[closed]).await, None);
    }
}
