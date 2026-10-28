//! `bookclub-server healthcheck`: the image's `HEALTHCHECK`. It asks the running server's
//! `GET /api/health` on the local address derived from `BIND_ADDR` and succeeds only on 200.
//!
//! It reads nothing but `BIND_ADDR`: no `Config::from_env` (which would fail on an unrelated
//! setting) and no database connection (the server's health route checks the database).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use anyhow::{Context, Result, bail};

/// `config.rs`'s default for `BIND_ADDR`.
const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8080";
/// The whole request, connect included. Below the image's `HEALTHCHECK --timeout`.
const TIMEOUT: Duration = Duration::from_secs(3);

/// Asks the local server for its health. `Ok` only when it answered 200.
pub async fn run() -> Result<()> {
    let url = health_url(std::env::var("BIND_ADDR").ok().as_deref())?;
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        // An HTTP_PROXY in the container's environment must not catch a request to itself.
        .no_proxy()
        .build()
        .context("cannot build the HTTP client")?;
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("GET {url} failed"))?;
    let status = response.status();
    if status != reqwest::StatusCode::OK {
        bail!("GET {url} answered {status}");
    }
    println!("GET {url} answered {status}");
    Ok(())
}

/// The health URL for `BIND_ADDR` (unset or empty: `config.rs`'s default). A wildcard address
/// is asked on the loopback address of its family: `0.0.0.0` on `127.0.0.1`, `[::]` on `[::1]`.
fn health_url(bind_addr: Option<&str>) -> Result<String> {
    let raw = bind_addr
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_BIND_ADDR);
    let mut addr: SocketAddr = raw
        .parse()
        .with_context(|| format!("BIND_ADDR {raw:?} is not a valid socket address"))?;
    if addr.ip().is_unspecified() {
        addr.set_ip(match addr.ip() {
            IpAddr::V4(_) => Ipv4Addr::LOCALHOST.into(),
            IpAddr::V6(_) => Ipv6Addr::LOCALHOST.into(),
        });
    }
    Ok(format!("http://{addr}/api/health"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_the_bind_address_and_loopback_for_a_wildcard() {
        for (bind_addr, url) in [
            (None, "http://127.0.0.1:8080/api/health"),
            (Some(""), "http://127.0.0.1:8080/api/health"),
            (Some(" 0.0.0.0:8080 "), "http://127.0.0.1:8080/api/health"),
            (Some("[::]:9000"), "http://[::1]:9000/api/health"),
            (Some("10.1.2.3:8081"), "http://10.1.2.3:8081/api/health"),
            (Some("[fd00::5]:8080"), "http://[fd00::5]:8080/api/health"),
        ] {
            assert_eq!(health_url(bind_addr).unwrap(), url, "{bind_addr:?}");
        }
    }

    #[test]
    fn rejects_a_bind_address_that_is_no_socket_address() {
        let error = health_url(Some("localhost:8080")).unwrap_err();
        assert!(error.to_string().contains("BIND_ADDR"), "{error}");
    }
}
