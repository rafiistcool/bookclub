//! The client address for rate limits (docs/phase1/conventions.md §10.2).

use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::HeaderMap;
use axum::http::request::Parts;
use ipnet::IpNet;

use crate::AppState;

/// The client's IP address. Never rejects.
///
/// It is the socket peer, or `0.0.0.0` when the server runs without `ConnectInfo` (tests).
/// `X-Forwarded-For` counts only when the peer is a `TRUSTED_PROXIES` address; then the
/// right-most address that is not itself trusted is the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientIp(pub IpAddr);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |info| info.0.ip());
        Ok(ClientIp(resolve(
            peer,
            &parts.headers,
            &state.config.trusted_proxies,
        )))
    }
}

/// The client address behind `peer`, given the trusted proxy networks.
pub fn resolve(peer: IpAddr, headers: &HeaderMap, trusted: &[IpNet]) -> IpAddr {
    let peer = peer.to_canonical();
    if !is_trusted(peer, trusted) {
        return peer;
    }
    let hops: Vec<&str> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .collect();
    let mut client = peer;
    for hop in hops.iter().rev() {
        let Ok(ip) = hop.parse::<IpAddr>().map(|ip| ip.to_canonical()) else {
            // Garbage in the chain: the last trusted hop is the best we know.
            break;
        };
        client = ip;
        if !is_trusted(ip, trusted) {
            break;
        }
    }
    client
}

fn is_trusted(ip: IpAddr, trusted: &[IpNet]) -> bool {
    trusted.iter().any(|net| net.contains(&ip))
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn nets(list: &[&str]) -> Vec<IpNet> {
        list.iter().map(|net| net.parse().unwrap()).collect()
    }

    fn forwarded(values: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append("x-forwarded-for", HeaderValue::from_str(value).unwrap());
        }
        headers
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn untrusted_peers_are_the_client() {
        let headers = forwarded(&["1.2.3.4"]);
        assert_eq!(resolve(ip("203.0.113.9"), &headers, &[]), ip("203.0.113.9"));
        assert_eq!(
            resolve(ip("203.0.113.9"), &headers, &nets(&["10.0.0.0/8"])),
            ip("203.0.113.9")
        );
    }

    #[test]
    fn a_trusted_proxy_forwards_the_client() {
        let trusted = nets(&["10.0.0.0/8"]);
        let headers = forwarded(&["198.51.100.1, 203.0.113.5"]);
        // Right-most untrusted: the client may forge the left part, the proxy appended the right.
        assert_eq!(
            resolve(ip("10.0.0.2"), &headers, &trusted),
            ip("203.0.113.5")
        );
    }

    #[test]
    fn trusted_hops_are_skipped_from_the_right() {
        let trusted = nets(&["10.0.0.0/8", "fd00::/8"]);
        let headers = forwarded(&["198.51.100.1", "203.0.113.5, 10.1.1.1", "fd00::7"]);
        assert_eq!(
            resolve(ip("10.0.0.2"), &headers, &trusted),
            ip("203.0.113.5")
        );
    }

    #[test]
    fn missing_or_garbage_headers_fall_back_to_the_nearest_trusted_hop() {
        let trusted = nets(&["10.0.0.0/8"]);
        assert_eq!(
            resolve(ip("10.0.0.2"), &HeaderMap::new(), &trusted),
            ip("10.0.0.2")
        );
        let headers = forwarded(&["203.0.113.5, not-an-ip, 10.9.9.9"]);
        assert_eq!(resolve(ip("10.0.0.2"), &headers, &trusted), ip("10.9.9.9"));
    }

    #[test]
    fn ipv4_mapped_peers_match_ipv4_networks() {
        let trusted = nets(&["127.0.0.1/32"]);
        let headers = forwarded(&["2001:db8::1"]);
        assert_eq!(
            resolve(ip("::ffff:127.0.0.1"), &headers, &trusted),
            ip("2001:db8::1")
        );
    }
}
