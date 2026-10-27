//! Settings read from environment variables at startup (docs/phase1/conventions.md §11).

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use ipnet::IpNet;
use url::Url;

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8080";
/// The Vite dev server origins, allowed for CORS and the origin check in DEV_MODE.
pub const VITE_ORIGINS: [&str; 2] = ["http://localhost:5173", "http://127.0.0.1:5173"];
const DEV_PUBLIC_URL: &str = "http://localhost:5173";
const DEFAULT_SERVICE_NAME: &str = "Bookclub";
const DEFAULT_MEDIA_DIR: &str = "./data/media";
const DEFAULT_OPENLIBRARY_URL: &str = "https://openlibrary.org";
const DEFAULT_OPENLIBRARY_COVERS_URL: &str = "https://covers.openlibrary.org";
const DEFAULT_DNB_SRU_URL: &str = "https://services.dnb.de/sru/dnb";
const DEFAULT_DNB_OAI_URL: &str = "https://services.dnb.de/oai/repository";
const DEFAULT_HARVEST_INTERVAL_MINUTES: u64 = 240;
const MIN_HARVEST_INTERVAL_MINUTES: u64 = 30;
const DEFAULT_HARVEST_SETS: &str = "dnb-all:reiheN,dnb:reiheA";
const DEFAULT_HARVEST_START_DAYS: u32 = 90;

/// Whether sign-up needs an invite (`SIGNUP_MODE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignupMode {
    /// Closed beta: sign-up needs a club invite or an admin's sign-up invite.
    Invite,
    /// Anyone may sign up (#55).
    Open,
}

impl SignupMode {
    /// The value `/api/config` reports: `"invite"` or `"open"`.
    pub fn as_str(self) -> &'static str {
        match self {
            SignupMode::Invite => "invite",
            SignupMode::Open => "open",
        }
    }
}

/// How mail leaves the server (`EMAIL_TRANSPORT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailTransport {
    /// Log every message (development).
    Log,
    /// Send through `SMTP_URL`.
    Smtp,
}

/// Settings read from environment variables at startup.
#[derive(Debug, Clone)]
pub struct Config {
    /// Postgres connection string (`DATABASE_URL`, required).
    pub database_url: String,
    /// Address the HTTP server listens on (`BIND_ADDR`, default `127.0.0.1:8080`).
    pub bind_addr: SocketAddr,
    /// Origin (`scheme://host[:port]`) for links in emails, the outbound User-Agent,
    /// the origin check and ICS (`PUBLIC_URL`).
    pub public_url: String,
    /// The service name in `/api/config` and email copy (`SERVICE_NAME`).
    pub service_name: String,
    /// Development defaults: insecure cookies, logged email, the Vite origin allowed (`DEV_MODE`).
    pub dev_mode: bool,
    /// Whether the session cookie carries `Secure` (`COOKIE_SECURE`, default `!DEV_MODE`).
    pub cookie_secure: bool,
    /// `SIGNUP_MODE`, default `invite`.
    pub signup_mode: SignupMode,
    /// Proxies whose `X-Forwarded-For` is trusted (`TRUSTED_PROXIES`; empty for `none`).
    pub trusted_proxies: Vec<IpNet>,
    /// Extra origins allowed for CORS and the origin check (`CORS_ORIGINS`).
    pub cors_origins: Vec<String>,
    /// The built SPA (`STATIC_DIR`); no SPA routes are served when unset.
    pub static_dir: Option<PathBuf>,
    /// Root of the filesystem media store (`MEDIA_DIR`, default `./data/media`).
    pub media_dir: PathBuf,
    /// `EMAIL_TRANSPORT`: `log` (default in DEV_MODE) or `smtp`.
    pub email_transport: EmailTransport,
    /// `SMTP_URL`, required for the smtp transport.
    pub smtp_url: Option<String>,
    /// `EMAIL_FROM`, the sender mailbox, required for the smtp transport.
    pub email_from: Option<String>,
    /// Contact address in the outbound User-Agent (`CONTACT_EMAIL`, required unless DEV_MODE).
    pub contact_email: Option<String>,
    /// Open Library API base URL (`OPENLIBRARY_URL`).
    pub openlibrary_url: Url,
    /// Open Library covers base URL (`OPENLIBRARY_COVERS_URL`).
    pub openlibrary_covers_url: Url,
    /// DNB SRU endpoint (`DNB_SRU_URL`).
    pub dnb_sru_url: Url,
    /// DNB OAI-PMH endpoint (`DNB_OAI_URL`).
    pub dnb_oai_url: Url,
    /// Run the DNB harvester job (`DNB_HARVEST`, default false).
    pub dnb_harvest: bool,
    /// Time between harvest runs (`DNB_HARVEST_INTERVAL_MINUTES`, default 240, at least 30).
    pub dnb_harvest_interval: Duration,
    /// OAI setSpecs to harvest (`DNB_HARVEST_SETS`).
    pub dnb_harvest_sets: Vec<String>,
    /// How many days back the first run of a set starts (`DNB_HARVEST_START_DAYS`, default 90).
    pub dnb_harvest_start_days: u32,
}

impl Config {
    /// Reads the configuration from the process environment.
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Reads the configuration through `lookup`, so tests can pass their own variables.
    /// Empty values count as unset. Invalid values fail with the variable name in the message.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let get = |key: &str| {
            lookup(key)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };

        let database_url = get("DATABASE_URL").context("DATABASE_URL is not set")?;
        let bind_addr = get("BIND_ADDR")
            .unwrap_or_else(|| DEFAULT_BIND_ADDR.to_owned())
            .parse()
            .context("BIND_ADDR is not a valid socket address, e.g. 0.0.0.0:8080")?;
        let dev_mode = parse_bool("DEV_MODE", get("DEV_MODE"))?.unwrap_or(false);

        let public_url = match get("PUBLIC_URL") {
            Some(raw) => {
                parse_origin(&raw).context("PUBLIC_URL must be an http(s) URL with a host")?
            }
            None if dev_mode => DEV_PUBLIC_URL.to_owned(),
            None => bail!("PUBLIC_URL is not set (it is only optional with DEV_MODE=true)"),
        };
        let cookie_secure = parse_bool("COOKIE_SECURE", get("COOKIE_SECURE"))?.unwrap_or(!dev_mode);
        let signup_mode = match get("SIGNUP_MODE").as_deref() {
            None | Some("invite") => SignupMode::Invite,
            Some("open") => SignupMode::Open,
            Some(other) => bail!("SIGNUP_MODE must be invite or open, not {other:?}"),
        };
        let trusted_proxies = match get("TRUSTED_PROXIES") {
            Some(raw) => parse_trusted_proxies(&raw)?,
            None if dev_mode => Vec::new(),
            None => bail!(
                "TRUSTED_PROXIES is not set: list the reverse proxy's CIDRs, or `none` when clients connect directly"
            ),
        };
        let cors_origins = match get("CORS_ORIGINS") {
            Some(raw) => split_list(&raw)
                .map(|origin| {
                    parse_any_origin(origin).with_context(|| {
                        format!("CORS_ORIGINS entry {origin:?} is not an origin like https://app.example.com")
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            None => Vec::new(),
        };
        let static_dir = get("STATIC_DIR").map(PathBuf::from);
        if let Some(dir) = &static_dir
            && !dir.is_dir()
        {
            bail!("STATIC_DIR {} is not a directory", dir.display());
        }
        let media_dir =
            PathBuf::from(get("MEDIA_DIR").unwrap_or_else(|| DEFAULT_MEDIA_DIR.to_owned()));

        let email_transport = match get("EMAIL_TRANSPORT").as_deref() {
            None if dev_mode => EmailTransport::Log,
            None | Some("smtp") => EmailTransport::Smtp,
            Some("log") => EmailTransport::Log,
            Some(other) => bail!("EMAIL_TRANSPORT must be log or smtp, not {other:?}"),
        };
        let smtp_url = get("SMTP_URL");
        let email_from = get("EMAIL_FROM");
        if email_transport == EmailTransport::Smtp {
            if smtp_url.is_none() {
                bail!("SMTP_URL is not set (needed for EMAIL_TRANSPORT=smtp)");
            }
            if email_from.is_none() {
                bail!("EMAIL_FROM is not set (needed for EMAIL_TRANSPORT=smtp)");
            }
        }
        let contact_email = get("CONTACT_EMAIL");
        if contact_email.is_none() && !dev_mode {
            bail!("CONTACT_EMAIL is not set (it goes into the User-Agent of catalog requests)");
        }

        let dnb_harvest = parse_bool("DNB_HARVEST", get("DNB_HARVEST"))?.unwrap_or(false);
        let interval_minutes = match get("DNB_HARVEST_INTERVAL_MINUTES") {
            Some(raw) => raw
                .parse::<u64>()
                .ok()
                .filter(|minutes| *minutes >= MIN_HARVEST_INTERVAL_MINUTES)
                .with_context(|| {
                    format!(
                        "DNB_HARVEST_INTERVAL_MINUTES must be a whole number of at least {MIN_HARVEST_INTERVAL_MINUTES}, not {raw:?}"
                    )
                })?,
            None => DEFAULT_HARVEST_INTERVAL_MINUTES,
        };
        let dnb_harvest_sets: Vec<String> = split_list(
            get("DNB_HARVEST_SETS")
                .as_deref()
                .unwrap_or(DEFAULT_HARVEST_SETS),
        )
        .map(str::to_owned)
        .collect();
        if dnb_harvest_sets.is_empty() {
            bail!("DNB_HARVEST_SETS must name at least one set");
        }
        let dnb_harvest_start_days = match get("DNB_HARVEST_START_DAYS") {
            Some(raw) => raw
                .parse::<u32>()
                .ok()
                .filter(|days| *days > 0)
                .with_context(|| {
                    format!("DNB_HARVEST_START_DAYS must be a positive whole number, not {raw:?}")
                })?,
            None => DEFAULT_HARVEST_START_DAYS,
        };

        Ok(Self {
            database_url,
            bind_addr,
            public_url,
            service_name: get("SERVICE_NAME").unwrap_or_else(|| DEFAULT_SERVICE_NAME.to_owned()),
            dev_mode,
            cookie_secure,
            signup_mode,
            trusted_proxies,
            cors_origins,
            static_dir,
            media_dir,
            email_transport,
            smtp_url,
            email_from,
            contact_email,
            openlibrary_url: parse_url(
                "OPENLIBRARY_URL",
                get("OPENLIBRARY_URL"),
                DEFAULT_OPENLIBRARY_URL,
            )?,
            openlibrary_covers_url: parse_url(
                "OPENLIBRARY_COVERS_URL",
                get("OPENLIBRARY_COVERS_URL"),
                DEFAULT_OPENLIBRARY_COVERS_URL,
            )?,
            dnb_sru_url: parse_url("DNB_SRU_URL", get("DNB_SRU_URL"), DEFAULT_DNB_SRU_URL)?,
            dnb_oai_url: parse_url("DNB_OAI_URL", get("DNB_OAI_URL"), DEFAULT_DNB_OAI_URL)?,
            dnb_harvest,
            dnb_harvest_interval: Duration::from_secs(interval_minutes * 60),
            dnb_harvest_sets,
            dnb_harvest_start_days,
        })
    }

    /// Origins whose cookie-authenticated unsafe requests pass the origin check (§5.8):
    /// `PUBLIC_URL`, `CORS_ORIGINS`, and the Vite origins in DEV_MODE.
    pub fn allowed_origins(&self) -> Vec<String> {
        let mut origins = vec![self.public_url.clone()];
        origins.extend(self.cors_origins.iter().cloned());
        if self.dev_mode {
            origins.extend(VITE_ORIGINS.iter().map(|origin| (*origin).to_owned()));
        }
        origins
    }

    /// Origins that get CORS headers: `CORS_ORIGINS`, plus the Vite origins in DEV_MODE.
    /// Empty means no CORS layer at all.
    pub fn cors_allowed_origins(&self) -> Vec<String> {
        let mut origins = self.cors_origins.clone();
        if self.dev_mode {
            origins.extend(VITE_ORIGINS.iter().map(|origin| (*origin).to_owned()));
        }
        origins
    }
}

fn split_list(raw: &str) -> impl Iterator<Item = &str> {
    raw.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
}

fn parse_bool(key: &str, value: Option<String>) -> Result<Option<bool>> {
    let Some(value) = value else {
        return Ok(None);
    };
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(Some(true)),
        "0" | "false" | "no" | "off" => Ok(Some(false)),
        _ => bail!("{key} must be true or false, not {value:?}"),
    }
}

/// Reduces an http(s) URL to its origin, `scheme://host[:port]`.
fn parse_origin(raw: &str) -> Result<String> {
    let url = Url::parse(raw)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("not an http(s) URL with a host");
    }
    Ok(url.origin().ascii_serialization())
}

/// Reduces a URL of any scheme with a host to `scheme://host[:port]`. The Phase 4 apps send
/// origins such as `capacitor://localhost`, which have no web origin of their own.
fn parse_any_origin(raw: &str) -> Result<String> {
    let url = Url::parse(raw)?;
    if matches!(url.scheme(), "http" | "https") {
        return parse_origin(raw);
    }
    let host = url.host_str().context("no host")?;
    Ok(match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    })
}

fn parse_url(key: &str, value: Option<String>, default: &str) -> Result<Url> {
    let raw = value.as_deref().unwrap_or(default);
    let url = Url::parse(raw).with_context(|| format!("{key} is not a valid URL: {raw:?}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("{key} must be an http(s) URL, not {raw:?}");
    }
    Ok(url)
}

fn parse_trusted_proxies(raw: &str) -> Result<Vec<IpNet>> {
    if raw.eq_ignore_ascii_case("none") {
        return Ok(Vec::new());
    }
    let nets = split_list(raw)
        .map(|item| {
            item.parse::<IpNet>()
                .or_else(|_| item.parse::<IpAddr>().map(IpNet::from))
                .with_context(|| {
                    format!("TRUSTED_PROXIES entry {item:?} is not a CIDR or an IP address")
                })
        })
        .collect::<Result<Vec<_>>>()?;
    if nets.is_empty() {
        bail!("TRUSTED_PROXIES must list CIDRs or be `none`");
    }
    Ok(nets)
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

    /// The minimal production environment.
    const PROD: &[(&str, &str)] = &[
        ("DATABASE_URL", "postgres://x"),
        ("PUBLIC_URL", "https://books.example.com/app/"),
        ("TRUSTED_PROXIES", "10.0.0.0/8, 192.168.1.5"),
        ("CONTACT_EMAIL", "ops@example.com"),
        ("SMTP_URL", "smtps://user:pass@smtp.example.eu:465"),
        ("EMAIL_FROM", "Bookclub <noreply@example.com>"),
    ];

    fn with(
        base: &[(&'static str, &'static str)],
        extra: &[(&'static str, &'static str)],
    ) -> Result<Config> {
        let mut pairs: Vec<(&str, &str)> = base
            .iter()
            .filter(|(k, _)| !extra.iter().any(|(e, _)| e == k))
            .copied()
            .collect();
        pairs.extend_from_slice(extra);
        Config::from_lookup(lookup_from(&pairs))
    }

    fn dev() -> Config {
        Config::from_lookup(lookup_from(&[
            ("DATABASE_URL", "postgres://x"),
            ("DEV_MODE", "true"),
        ]))
        .unwrap()
    }

    #[test]
    fn requires_database_url() {
        let error = Config::from_lookup(lookup_from(&[])).unwrap_err();
        assert!(error.to_string().contains("DATABASE_URL"));
    }

    #[test]
    fn defaults_bind_addr_to_loopback() {
        assert_eq!(dev().bind_addr, "127.0.0.1:8080".parse().unwrap());
    }

    #[test]
    fn rejects_invalid_bind_addr() {
        let error = with(PROD, &[("BIND_ADDR", "not-an-address")]).unwrap_err();
        assert!(error.to_string().contains("BIND_ADDR"));
    }

    #[test]
    fn dev_mode_defaults() {
        let config = dev();
        assert!(config.dev_mode);
        assert!(!config.cookie_secure);
        assert_eq!(config.public_url, "http://localhost:5173");
        assert_eq!(config.email_transport, EmailTransport::Log);
        assert!(config.trusted_proxies.is_empty());
        assert_eq!(config.contact_email, None);
        assert_eq!(config.signup_mode, SignupMode::Invite);
        assert_eq!(config.service_name, "Bookclub");
        assert_eq!(config.media_dir, PathBuf::from("./data/media"));
        assert_eq!(config.static_dir, None);
        assert!(!config.dnb_harvest);
        assert_eq!(config.dnb_harvest_interval, Duration::from_secs(240 * 60));
        assert_eq!(config.dnb_harvest_sets, ["dnb-all:reiheN", "dnb:reiheA"]);
        assert_eq!(config.dnb_harvest_start_days, 90);
        assert_eq!(config.openlibrary_url.as_str(), "https://openlibrary.org/");
        assert_eq!(
            config.dnb_sru_url.as_str(),
            "https://services.dnb.de/sru/dnb"
        );
    }

    #[test]
    fn production_reads_every_required_variable() {
        let config = with(PROD, &[]).unwrap();
        assert!(!config.dev_mode);
        assert!(config.cookie_secure);
        assert_eq!(config.email_transport, EmailTransport::Smtp);
        assert_eq!(config.public_url, "https://books.example.com");
        assert_eq!(
            config.trusted_proxies,
            vec![
                "10.0.0.0/8".parse::<IpNet>().unwrap(),
                "192.168.1.5/32".parse::<IpNet>().unwrap()
            ]
        );
        assert_eq!(config.contact_email.as_deref(), Some("ops@example.com"));
    }

    #[test]
    fn production_requires_public_url_trusted_proxies_and_contact() {
        for key in [
            "PUBLIC_URL",
            "TRUSTED_PROXIES",
            "CONTACT_EMAIL",
            "SMTP_URL",
            "EMAIL_FROM",
        ] {
            let pairs: Vec<(&str, &str)> =
                PROD.iter().filter(|(k, _)| *k != key).copied().collect();
            let error = Config::from_lookup(lookup_from(&pairs)).unwrap_err();
            assert!(error.to_string().contains(key), "{key}: {error}");
        }
    }

    #[test]
    fn trusted_proxies_none_means_direct_clients() {
        let config = with(PROD, &[("TRUSTED_PROXIES", "none")]).unwrap();
        assert!(config.trusted_proxies.is_empty());
        let error = with(PROD, &[("TRUSTED_PROXIES", "10.0.0.0/8,not-a-net")]).unwrap_err();
        assert!(error.to_string().contains("TRUSTED_PROXIES"));
    }

    #[test]
    fn public_url_is_reduced_to_its_origin_and_validated() {
        let config = with(PROD, &[("PUBLIC_URL", "http://agents.lan:5173/x?y")]).unwrap();
        assert_eq!(config.public_url, "http://agents.lan:5173");
        for bad in ["not-a-url", "ftp://example.com", "mailto:x@example.com"] {
            let error = with(PROD, &[("PUBLIC_URL", bad)]).unwrap_err();
            assert!(error.to_string().contains("PUBLIC_URL"), "{bad}: {error}");
        }
    }

    #[test]
    fn cors_origins_are_normalised() {
        let config = with(
            PROD,
            &[(
                "CORS_ORIGINS",
                "capacitor://localhost, https://app.example.com/, http://x.test:81",
            )],
        )
        .unwrap();
        assert_eq!(
            config.cors_origins,
            [
                "capacitor://localhost",
                "https://app.example.com",
                "http://x.test:81"
            ]
        );
        assert_eq!(
            config.allowed_origins(),
            [
                "https://books.example.com",
                "capacitor://localhost",
                "https://app.example.com",
                "http://x.test:81"
            ]
        );
        let error = with(PROD, &[("CORS_ORIGINS", "not an origin")]).unwrap_err();
        assert!(error.to_string().contains("CORS_ORIGINS"));
    }

    #[test]
    fn dev_mode_allows_the_vite_origin() {
        let config = dev();
        assert!(
            config
                .allowed_origins()
                .contains(&"http://localhost:5173".to_owned())
        );
        assert!(
            config
                .cors_allowed_origins()
                .contains(&"http://127.0.0.1:5173".to_owned())
        );
        assert!(with(PROD, &[]).unwrap().cors_allowed_origins().is_empty());
    }

    #[test]
    fn rejects_bad_enums_and_bools() {
        for (key, value) in [
            ("SIGNUP_MODE", "closed"),
            ("EMAIL_TRANSPORT", "carrier-pigeon"),
            ("DEV_MODE", "maybe"),
            ("COOKIE_SECURE", "2"),
            ("DNB_HARVEST", "sometimes"),
        ] {
            let error = with(PROD, &[(key, value)]).unwrap_err();
            assert!(error.to_string().contains(key), "{key}: {error}");
        }
        let config = with(PROD, &[("SIGNUP_MODE", "open"), ("COOKIE_SECURE", "false")]).unwrap();
        assert_eq!(config.signup_mode, SignupMode::Open);
        assert!(!config.cookie_secure);
    }

    #[test]
    fn harvest_settings_are_validated() {
        let error = with(PROD, &[("DNB_HARVEST_INTERVAL_MINUTES", "29")]).unwrap_err();
        assert!(error.to_string().contains("DNB_HARVEST_INTERVAL_MINUTES"));
        let error = with(PROD, &[("DNB_HARVEST_START_DAYS", "0")]).unwrap_err();
        assert!(error.to_string().contains("DNB_HARVEST_START_DAYS"));
        let config = with(
            PROD,
            &[
                ("DNB_HARVEST", "true"),
                ("DNB_HARVEST_INTERVAL_MINUTES", "30"),
                ("DNB_HARVEST_SETS", " dnb:reiheA ,"),
                ("DNB_HARVEST_START_DAYS", "7"),
            ],
        )
        .unwrap();
        assert!(config.dnb_harvest);
        assert_eq!(config.dnb_harvest_interval, Duration::from_secs(30 * 60));
        assert_eq!(config.dnb_harvest_sets, ["dnb:reiheA"]);
        assert_eq!(config.dnb_harvest_start_days, 7);
    }

    #[test]
    fn upstream_urls_can_point_at_a_fake() {
        let config = with(PROD, &[("OPENLIBRARY_URL", "http://127.0.0.1:9999")]).unwrap();
        assert_eq!(config.openlibrary_url.as_str(), "http://127.0.0.1:9999/");
        let error = with(PROD, &[("DNB_OAI_URL", "nope")]).unwrap_err();
        assert!(error.to_string().contains("DNB_OAI_URL"));
    }

    #[test]
    fn static_dir_must_exist() {
        let error = with(PROD, &[("STATIC_DIR", "/definitely/not/here")]).unwrap_err();
        assert!(error.to_string().contains("STATIC_DIR"));
    }
}
