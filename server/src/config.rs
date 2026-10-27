use std::net::SocketAddr;

use anyhow::{Context, Result};

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8080";

/// Settings read from environment variables at startup.
#[derive(Debug, Clone)]
pub struct Config {
    /// Postgres connection string (`DATABASE_URL`, required).
    pub database_url: String,
    /// Address the HTTP server listens on (`BIND_ADDR`, default `127.0.0.1:8080`).
    pub bind_addr: SocketAddr,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let database_url = lookup("DATABASE_URL").context("DATABASE_URL is not set")?;
        let bind_addr = lookup("BIND_ADDR")
            .unwrap_or_else(|| DEFAULT_BIND_ADDR.to_owned())
            .parse()
            .context("BIND_ADDR is not a valid socket address, e.g. 0.0.0.0:8080")?;
        Ok(Self {
            database_url,
            bind_addr,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    #[test]
    fn requires_database_url() {
        let error = Config::from_lookup(lookup_from(&[])).unwrap_err();
        assert!(error.to_string().contains("DATABASE_URL"));
    }

    #[test]
    fn defaults_bind_addr_to_loopback() {
        let config = Config::from_lookup(lookup_from(&[("DATABASE_URL", "postgres://x")])).unwrap();
        assert_eq!(config.bind_addr, "127.0.0.1:8080".parse().unwrap());
    }

    #[test]
    fn rejects_invalid_bind_addr() {
        let error = Config::from_lookup(lookup_from(&[
            ("DATABASE_URL", "postgres://x"),
            ("BIND_ADDR", "not-an-address"),
        ]))
        .unwrap_err();
        assert!(error.to_string().contains("BIND_ADDR"));
    }
}
