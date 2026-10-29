//! The deployment contract (#51), checked without Docker:
//! - `deploy/hosted/env.example` names every variable the server reads, nothing it does not
//!   know, and every variable `deploy/hosted/compose.yaml` uses; filled in, it is a valid
//!   production configuration.
//! - `deploy/hosted/ci.env`, which CI's image smoke test boots, is one as it stands.
//! - The image's commands, through the real binary: `healthcheck` succeeds only on a 200 from
//!   `/api/health`, `send-test-mail` delivers one message with a `Message-ID` on the sender's
//!   domain and fails on an SMTP error, and `serve` and `import-legacy` refuse a `MEDIA_DIR`
//!   they cannot write before they touch the database.
//! - `deploy/hosted/smoke-test.sh` refuses names that are not CI's before it runs `docker`.
//! - Every action the Forgejo workflows use is pinned to a commit.
//! - `deploy/hosted/backup.env.example` names every setting of the backup and update scripts,
//!   and every systemd unit starts a script that exists.
//! - `deploy/hosted/update.sh`, with `docker` and `curl` stood in for: it backs up before it
//!   replaces the app, never touches an app that is up to date or stopped, and pings the heartbeat
//!   with the outcome.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::{Duration, Instant};

use bookclub_server::config::{Config, EmailTransport};
use regex::Regex;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Read by `tracing_subscriber`'s `EnvFilter` in `main.rs`, not through `config.rs`.
const READ_OUTSIDE_CONFIG: &[&str] = &["RUST_LOG"];
/// Set (not only shown commented out) in `env.example`: what production needs plus the values
/// DEP fixes (docs/phase2/work-plan.md, DEP).
const SET_IN_ENV_EXAMPLE: &[&str] = &[
    "POSTGRES_PASSWORD",
    "DATABASE_URL",
    "PUBLIC_URL",
    "SERVICE_NAME",
    "TRUSTED_PROXIES",
    "CONTACT_EMAIL",
    "SMTP_URL",
    "EMAIL_FROM",
    "SIGNUP_MODE",
    "LEGAL_DIR",
    "DNB_HARVEST",
];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}

fn read(relative: &str) -> String {
    std::fs::read_to_string(repo().join(relative))
        .unwrap_or_else(|error| panic!("cannot read {relative}: {error}"))
}

fn names(pattern: &str, text: &str) -> BTreeSet<String> {
    Regex::new(pattern)
        .unwrap()
        .captures_iter(text)
        .map(|captures| captures[1].to_owned())
        .collect()
}

/// Every variable the server reads: `config.rs` reads each one through `get("NAME")`, and any
/// other module would use `std::env::var("NAME")`.
fn server_variables() -> BTreeSet<String> {
    let mut found = names(
        r#"\bget\(\s*"([A-Z][A-Z0-9_]*)"\s*\)"#,
        &read("server/src/config.rs"),
    );
    let mut sources = Vec::new();
    rust_files(&repo().join("server/src"), &mut sources);
    for file in sources {
        let text = std::fs::read_to_string(&file).unwrap();
        found.extend(names(
            r#"env::var(?:_os)?\(\s*"([A-Z][A-Z0-9_]*)"\s*\)"#,
            &text,
        ));
    }
    found.extend(READ_OUTSIDE_CONFIG.iter().map(|name| (*name).to_owned()));
    found
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// An env file as `NAME → (set, value)`: `NAME=value` is set, `#NAME=value` (no space after
/// the `#`) is shown commented out. Prose comments start with `# `.
fn env_file(relative: &str) -> BTreeMap<String, (bool, String)> {
    let line = Regex::new(r"^(#?)([A-Z][A-Z0-9_]*)=(.*)$").unwrap();
    let mut entries = BTreeMap::new();
    for text in read(relative).lines() {
        let Some(captures) = line.captures(text) else {
            continue;
        };
        let name = captures[2].to_owned();
        let entry = (captures[1].is_empty(), captures[3].to_owned());
        assert!(
            entries.insert(name.clone(), entry).is_none(),
            "{relative} names {name} twice"
        );
    }
    entries
}

fn env_example() -> BTreeMap<String, (bool, String)> {
    env_file("deploy/hosted/env.example")
}

fn env_example_values() -> BTreeMap<String, String> {
    values(&env_example())
}

/// The set values of an env file, with `${NAME}` replaced from the same file as compose does.
fn values(entries: &BTreeMap<String, (bool, String)>) -> BTreeMap<String, String> {
    let reference = Regex::new(r"\$\{([A-Z][A-Z0-9_]*)\}").unwrap();
    entries
        .iter()
        .filter(|(_, (set, _))| *set)
        .map(|(name, (_, value))| {
            let value = reference.replace_all(value, |captures: &regex::Captures| {
                let (set, value) = &entries[&captures[1]];
                assert!(set, "{name} refers to {}, which is not set", &captures[1]);
                value.clone()
            });
            (name.clone(), value.into_owned())
        })
        .collect()
}

#[test]
fn env_example_names_every_variable_the_server_reads() {
    let documented = env_example();
    let missing: Vec<_> = server_variables()
        .into_iter()
        .filter(|name| !documented.contains_key(name))
        .collect();
    assert!(
        missing.is_empty(),
        "deploy/hosted/env.example does not name {missing:?}: add them, commented out \
         (#NAME=default) when production keeps the default"
    );
}

#[test]
fn env_example_names_only_variables_someone_reads() {
    let compose = compose_variables();
    let server = server_variables();
    let unknown: Vec<_> = env_example()
        .into_keys()
        .filter(|name| !server.contains(name) && !compose.contains(name))
        .collect();
    assert!(
        unknown.is_empty(),
        "deploy/hosted/env.example names {unknown:?}, which neither the server nor compose.yaml \
         reads (a typo?)"
    );
}

fn compose_variables() -> BTreeSet<String> {
    names(
        r"\$\{([A-Z][A-Z0-9_]*)",
        &read("deploy/hosted/compose.yaml"),
    )
}

#[test]
fn env_example_names_every_compose_variable() {
    let documented = env_example();
    let missing: Vec<_> = compose_variables()
        .into_iter()
        .filter(|name| !documented.contains_key(name))
        .collect();
    assert!(
        missing.is_empty(),
        "compose.yaml uses {missing:?}, which env.example does not name"
    );
}

#[test]
fn env_example_sets_what_production_needs() {
    let entries = env_example();
    for name in SET_IN_ENV_EXAMPLE {
        assert!(
            entries.get(*name).is_some_and(|(set, _)| *set),
            "env.example must set {name}, not only show it"
        );
    }
    let values = env_example_values();
    assert_eq!(
        values["DNB_HARVEST"], "false",
        "harvest only after the import"
    );
    assert_eq!(
        values["LEGAL_DIR"], "/app/legal",
        "compose.yaml mounts it there"
    );
    assert_eq!(
        values["DATABASE_URL"],
        "postgres://bookclub:CHANGE_ME@postgres:5432/bookclub"
    );
}

/// Filled in, `env.example` is a production configuration (no DEV_MODE, Secure cookies, SMTP);
/// as shipped, it does not start.
#[test]
fn env_example_filled_in_is_a_production_configuration() {
    let values = env_example_values();
    let shipped = Config::from_lookup(|key| values.get(key).cloned()).unwrap_err();
    assert!(shipped.to_string().contains("TRUSTED_PROXIES"), "{shipped}");

    let mut filled = values.clone();
    filled.insert("TRUSTED_PROXIES".into(), "192.0.2.10".into());
    let config = Config::from_lookup(|key| filled.get(key).cloned()).unwrap();
    assert!(!config.dev_mode);
    assert!(config.cookie_secure);
    assert_eq!(config.email_transport, EmailTransport::Smtp);
    assert!(!config.dnb_harvest);
    assert_eq!(config.service_name, "Bookclub");
}

/// `ci.env` boots compose.yaml in CI's image smoke test (`deploy/hosted/smoke-test.sh`). It
/// names nothing env.example does not (apart from compose's own project name), points compose
/// at itself, sets what production needs, and the server accepts it as it stands: no DEV_MODE,
/// so the smoke test boots the production code paths.
#[test]
fn ci_env_is_a_production_configuration() {
    let documented = env_example();
    let ci = env_file("deploy/hosted/ci.env");
    let unknown: Vec<_> = ci
        .keys()
        .filter(|name| !documented.contains_key(*name) && *name != "COMPOSE_PROJECT_NAME")
        .collect();
    assert!(
        unknown.is_empty(),
        "ci.env names {unknown:?}, which env.example does not"
    );
    assert!(ci.values().all(|(set, _)| *set), "ci.env only sets values");

    let values = values(&ci);
    assert_eq!(values["BOOKCLUB_ENV_FILE"], "ci.env");
    assert_ne!(
        values["COMPOSE_PROJECT_NAME"], "bookclub",
        "the production project may run on the same daemon"
    );
    assert!(
        values["BOOKCLUB_DATA_DIR"].starts_with('/'),
        "a path on the daemon's host, not one compose resolves on the job's file system"
    );
    assert_eq!(
        values["BOOKCLUB_RESTART"], "no",
        "a stack a cancelled run left behind must not come back with the daemon"
    );
    for name in compose_variables() {
        assert!(values.contains_key(&name), "ci.env does not set {name}");
    }
    for name in SET_IN_ENV_EXAMPLE {
        assert!(values.contains_key(*name), "ci.env does not set {name}");
    }

    let config = Config::from_lookup(|key| values.get(key).cloned()).unwrap();
    assert!(!config.dev_mode);
    assert_eq!(config.email_transport, EmailTransport::Smtp);
    assert!(!config.dnb_harvest);
}

// ---- The binary -----------------------------------------------------------------------------

/// The server binary with an empty environment.
fn bin() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bookclub-server"));
    command
        .env_clear()
        .env("RUST_LOG", "error")
        .kill_on_drop(true);
    command
}

/// Runs `command`, failing the test after `limit`.
async fn run(command: &mut Command, limit: Duration) -> Output {
    tokio::time::timeout(limit, command.output())
        .await
        .unwrap_or_else(|_| panic!("the command did not finish within {limit:?}"))
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Nothing listens there.
const UNREACHABLE_DB: &str = "postgres://bookclub:x@127.0.0.1:1/bookclub";

/// Where the healthcheck's environment points HTTP proxies: nothing listens there, so a request
/// that went through the proxy would fail.
const DEAD_PROXY: &str = "http://127.0.0.1:1";

/// `bookclub-server healthcheck` with `BIND_ADDR`, and with a proxy in the environment that it
/// must not use: a container may have HTTP_PROXY set, and reqwest does not bypass loopback on
/// its own.
async fn healthcheck(bind_addr: &str) -> (Output, Duration) {
    let started = Instant::now();
    let output = run(
        bin()
            .arg("healthcheck")
            .env("BIND_ADDR", bind_addr)
            .env("HTTP_PROXY", DEAD_PROXY)
            .env("http_proxy", DEAD_PROXY)
            .env("ALL_PROXY", DEAD_PROXY),
        Duration::from_secs(20),
    )
    .await;
    (output, started.elapsed())
}

/// The wildcard bind address is asked on loopback, and only a 200 counts.
#[tokio::test]
async fn healthcheck_succeeds_only_on_a_200() {
    let server = MockServer::start().await;
    let port = server.address().port();
    let bind_addr = format!("0.0.0.0:{port}");

    Mock::given(method("GET"))
        .and(path("/api/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "ok"})))
        .mount(&server)
        .await;
    let (output, _) = healthcheck(&bind_addr).await;
    assert!(output.status.success(), "{}", stderr(&output));

    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/api/health"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({"status": "unavailable"})))
        .mount(&server)
        .await;
    let (output, _) = healthcheck(&bind_addr).await;
    assert!(!output.status.success());
    assert!(stderr(&output).contains("503"), "{}", stderr(&output));
}

#[tokio::test]
async fn healthcheck_gives_up_after_three_seconds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/health"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
        .mount(&server)
        .await;
    let (output, elapsed) = healthcheck(&server.address().to_string()).await;
    assert!(!output.status.success());
    assert!(
        elapsed >= Duration::from_secs(3) && elapsed < Duration::from_secs(10),
        "{elapsed:?}"
    );
}

#[tokio::test]
async fn healthcheck_fails_when_nothing_listens() {
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let (output, _) = healthcheck(&format!("127.0.0.1:{port}")).await;
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("/api/health"),
        "{}",
        stderr(&output)
    );
}

/// A one-connection SMTP server. It accepts one message, or refuses every recipient with 550,
/// and returns the message as received (dot-stuffing and all).
async fn fake_smtp(refuse_recipients: bool) -> (u16, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut lines = BufReader::new(read).lines();
        let mut data = String::new();
        write.write_all(b"220 fake ESMTP\r\n").await.unwrap();
        while let Ok(Some(line)) = lines.next_line().await {
            let verb = line.get(..4).unwrap_or_default().to_ascii_uppercase();
            let reply: &[u8] = match verb.as_str() {
                "EHLO" | "HELO" => b"250 fake\r\n",
                "RCPT" if refuse_recipients => b"550 5.1.1 no such mailbox\r\n",
                "DATA" => {
                    write.write_all(b"354 go ahead\r\n").await.unwrap();
                    while let Ok(Some(line)) = lines.next_line().await {
                        if line == "." {
                            break;
                        }
                        data.push_str(&line);
                        data.push('\n');
                    }
                    b"250 2.0.0 queued\r\n"
                }
                "QUIT" => b"221 bye\r\n",
                _ => b"250 ok\r\n",
            };
            if write.write_all(reply).await.is_err() || verb == "QUIT" {
                break;
            }
        }
        data
    });
    (port, task)
}

/// `send-test-mail` with a production configuration (no DEV_MODE) that sends to `port` on
/// loopback, which `config.rs` allows without TLS. The database is never contacted.
fn send_test_mail(port: u16, to: &str) -> Command {
    let mut command = bin();
    command
        .args(["send-test-mail", to])
        .env("DATABASE_URL", UNREACHABLE_DB)
        .env("PUBLIC_URL", "https://books.example.com")
        .env("TRUSTED_PROXIES", "none")
        .env("CONTACT_EMAIL", "contact@books.example.com")
        .env("SERVICE_NAME", "Shelfmates")
        .env("SMTP_URL", format!("smtp://127.0.0.1:{port}"))
        .env("EMAIL_FROM", "Shelfmates <noreply@mail.example.com>");
    command
}

#[tokio::test]
async fn send_test_mail_delivers_one_message_with_a_message_id_on_the_sender_domain() {
    let (port, received) = fake_smtp(false).await;
    let output = run(
        &mut send_test_mail(port, "ada@example.test"),
        Duration::from_secs(30),
    )
    .await;
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "sent a test mail to ada@example.test\n"
    );
    let message = tokio::time::timeout(Duration::from_secs(10), received)
        .await
        .unwrap()
        .unwrap();
    let header = |name: &str| {
        message
            .lines()
            .take_while(|line| !line.is_empty())
            .find_map(|line| line.strip_prefix(&format!("{name}: ")))
            .unwrap_or_else(|| panic!("no {name} header in\n{message}"))
            .to_owned()
    };
    assert!(
        Regex::new(r"^<[0-9a-f-]{36}@mail\.example\.com>$")
            .unwrap()
            .is_match(&header("Message-ID")),
        "{message}"
    );
    assert!(!header("Date").is_empty());
    assert_eq!(header("From"), "Shelfmates <noreply@mail.example.com>");
    assert_eq!(header("To"), "ada@example.test");
    assert_eq!(header("Subject"), "Shelfmates test mail");
    assert!(
        message.contains("This is a test mail from Shelfmates at https://books.example.com."),
        "{message}"
    );
}

#[tokio::test]
async fn send_test_mail_fails_on_an_smtp_error() {
    let (port, _received) = fake_smtp(true).await;
    let output = run(
        &mut send_test_mail(port, "ada@example.test"),
        Duration::from_secs(30),
    )
    .await;
    assert!(!output.status.success());
    let stderr = stderr(&output);
    assert!(stderr.contains("SMTP send failed"), "{stderr}");
    assert!(stderr.contains("550"), "{stderr}");
}

/// The log goes to stderr without colour codes, so stdout holds only the command's result
/// (`beta-invite`'s code and link, for one). DEV_MODE's log transport logs the mail at INFO.
#[tokio::test]
async fn the_log_goes_to_stderr_and_stdout_holds_only_the_result() {
    let output = run(
        bin()
            .args(["send-test-mail", "ada@example.test"])
            .env("RUST_LOG", "info")
            .env("DEV_MODE", "true")
            .env("DATABASE_URL", UNREACHABLE_DB),
        Duration::from_secs(30),
    )
    .await;
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "sent a test mail to ada@example.test\n"
    );
    let stderr = stderr(&output);
    assert!(stderr.contains("email (log transport)"), "{stderr}");
    assert!(!stderr.contains('\u{1b}'), "{stderr}");
}

#[tokio::test]
async fn send_test_mail_refuses_what_is_no_plain_address() {
    let (port, _received) = fake_smtp(false).await;
    let output = run(
        &mut send_test_mail(port, "Ada <ada@example.test>"),
        Duration::from_secs(30),
    )
    .await;
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("is not a plain email address"),
        "{}",
        stderr(&output)
    );
}

/// The check runs before the database connection, which would retry for 60 s here. Two kinds
/// of `MEDIA_DIR`: one below a regular file, which cannot be created, and `/proc`, which exists
/// but takes no new file even from uid 0 (as a root-owned bind mount is for the image's uid
/// 1000); only the probe file catches the second.
#[tokio::test]
async fn serve_and_import_legacy_refuse_a_media_dir_they_cannot_write() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut media_dirs = vec![file.path().join("media")];
    if cfg!(target_os = "linux") {
        media_dirs.push(PathBuf::from("/proc"));
    }
    for media_dir in media_dirs {
        refuse_media_dir(&media_dir).await;
    }
}

async fn refuse_media_dir(media_dir: &Path) {
    let expected = format!("MEDIA_DIR {} is not writable", media_dir.display());
    let import = [
        "import-legacy",
        "bookclub.db",
        "--owner",
        "ada",
        "--club-name",
        "Thursday Readers",
        "--timezone",
        "Europe/Berlin",
    ];
    for args in [&["serve"][..], &import[..]] {
        let output = run(
            bin()
                .args(args)
                .env("DEV_MODE", "true")
                .env("DATABASE_URL", UNREACHABLE_DB)
                .env("MEDIA_DIR", media_dir),
            Duration::from_secs(20),
        )
        .await;
        assert!(!output.status.success(), "{args:?}");
        assert!(
            stderr(&output).contains(&expected),
            "{args:?}: {}",
            stderr(&output)
        );
    }
}

// ---- smoke-test.sh --------------------------------------------------------------------------

/// `deploy/hosted/smoke-test.sh --down` with `vars` over ci.env's values. `docker` is a bash
/// function (imported from the environment) that only records that it ran, so nothing reaches a
/// real daemon. Returns the output and whether `docker` ran.
fn smoke_test_down(vars: &[(&str, &str)]) -> (Output, bool) {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("docker-ran");
    let mut command = std::process::Command::new("bash");
    command
        .arg(repo().join("deploy/hosted/smoke-test.sh"))
        .arg("--down")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("SMOKE_TEST_MARKER", &marker)
        .env("BASH_FUNC_docker%%", r#"() { : >"$SMOKE_TEST_MARKER"; }"#);
    for (name, value) in vars {
        command.env(name, value);
    }
    let output = command.output().unwrap();
    (output, marker.exists())
}

/// The teardown removes the data directory as root, the compose project with its volumes, and
/// the image tag that the environment names. A shell that loaded the production `.env` must not
/// turn that against production, so the script refuses names that are not CI's before it runs
/// a single `docker` command.
#[test]
fn smoke_test_refuses_names_that_are_not_ci_names() {
    let (output, docker_ran) = smoke_test_down(&[]);
    assert!(docker_ran, "ci.env's own names pass: {}", stderr(&output));
    let (output, docker_ran) = smoke_test_down(&[
        ("COMPOSE_PROJECT_NAME", "bookclub-ci-4711"),
        ("BOOKCLUB_TAG", "ci-4711"),
        ("BOOKCLUB_DATA_DIR", "/tmp/bookclub-ci-4711/"),
    ]);
    assert!(docker_ran, "CI's per-run names pass: {}", stderr(&output));

    for (name, value) in [
        ("BOOKCLUB_DATA_DIR", "/srv/bookclub"),
        ("BOOKCLUB_DATA_DIR", "/tmp/"),
        ("BOOKCLUB_DATA_DIR", "/tmp"),
        ("BOOKCLUB_DATA_DIR", "/tmp/bookclub-ci-1/media"),
        ("BOOKCLUB_DATA_DIR", "/tmp/bookclub-ci-1/.."),
        ("BOOKCLUB_DATA_DIR", "/tmp/../srv/bookclub-ci"),
        ("COMPOSE_PROJECT_NAME", "bookclub"),
        ("COMPOSE_PROJECT_NAME", "bookclub-staging"),
        ("BOOKCLUB_TAG", "stable"),
        ("BOOKCLUB_TAG", "0.11.0"),
    ] {
        let (output, docker_ran) = smoke_test_down(&[(name, value)]);
        assert!(!output.status.success(), "{name}={value}");
        assert!(!docker_ran, "{name}={value} reached docker");
        let stderr = stderr(&output);
        assert!(
            stderr.contains(&format!("{name}=")) && stderr.contains("is not a CI"),
            "{name}={value}: {stderr}"
        );
    }
}

/// Every `uses:` in `.forgejo/workflows/` names a full commit, with its release in a comment
/// (security review, Phase 2 Wave 1 #2): a tag or a branch can be moved to other code, and the
/// release job hands its actions the registry token and the Docker daemon of the runner's host.
/// An action in this repository (`./…`) is code the workflow checks out anyway.
#[test]
fn workflows_pin_every_action_to_a_commit() {
    let uses = Regex::new(r"^\s*(?:-\s+)?uses:\s*(.*?)\s*$").unwrap();
    let pinned = Regex::new(r"^[\w.-]+/[\w./-]+@[0-9a-f]{40} # \S").unwrap();
    let dir = repo().join(".forgejo/workflows");
    let mut workflows: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext == "yml" || ext == "yaml")
        })
        .collect();
    workflows.sort();
    assert!(workflows.len() >= 2, "{workflows:?}");
    for workflow in &workflows {
        let text = std::fs::read_to_string(workflow).unwrap();
        let mut checked = 0;
        for (index, line) in text.lines().enumerate() {
            let Some(captures) = uses.captures(line) else {
                continue;
            };
            let action = &captures[1];
            checked += 1;
            assert!(
                action.starts_with("./") || pinned.is_match(action),
                "{}:{}: `{action}` is not pinned to a commit with its release in a comment",
                workflow.display(),
                index + 1
            );
        }
        assert!(checked > 0, "no `uses:` found in {}", workflow.display());
    }
}

// ---- The backup and update scripts ----------------------------------------------------------

/// The settings `load_config` in `backup-lib.sh` reads: every `${NAME…}` in its body, except
/// `BACKUP_ENV`, which names the settings file itself.
fn script_settings() -> BTreeSet<String> {
    let lib = read("deploy/hosted/backup-lib.sh");
    let start = lib
        .find("load_config() {")
        .expect("backup-lib.sh has no load_config");
    let end = start + lib[start..].find("\n}\n").expect("load_config has no end");
    let mut found = names(r"\$\{([A-Z][A-Z0-9_]*)", &lib[start..end]);
    found.remove("BACKUP_ENV");
    found
}

/// `backup.env.example` names every setting of `backup.sh`, `restore-test.sh` and `update.sh`
/// (set, or shown as `#NAME=`), and nothing else besides the variables restic reads itself.
#[test]
fn backup_env_example_names_every_script_setting() {
    let example = env_file("deploy/hosted/backup.env.example");
    let settings = script_settings();
    for name in ["COMPOSE_DIR", "RESTIC_REPOSITORY", "UPDATE_HEARTBEAT_URL"] {
        assert!(settings.contains(name), "load_config does not read {name}");
    }
    let missing: Vec<_> = settings
        .iter()
        .filter(|name| !example.contains_key(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "backup-lib.sh reads {missing:?}, which backup.env.example does not name"
    );
    let unknown: Vec<_> = example
        .keys()
        .filter(|name| {
            !settings.contains(*name) && !name.starts_with("RESTIC_") && !name.starts_with("AWS_")
        })
        .collect();
    assert!(
        unknown.is_empty(),
        "backup.env.example names {unknown:?}, which no script reads (a typo?)"
    );
}

/// Every timer in `deploy/hosted/systemd/` starts a service of the same name, and every service
/// starts a script of `deploy/hosted/` (installed in `/opt/bookclub`) that exists and is
/// executable.
#[cfg(unix)]
#[test]
fn systemd_units_start_scripts_that_exist() {
    use std::os::unix::fs::PermissionsExt;

    let dir = repo().join("deploy/hosted/systemd");
    let exec_start = Regex::new(r"(?m)^ExecStart=/opt/bookclub/([\w.-]+)$").unwrap();
    let mut units: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    units.sort();
    let mut scripts = BTreeSet::new();
    for unit in &units {
        if let Some(name) = unit.strip_suffix(".timer") {
            assert!(
                units.contains(&format!("{name}.service")),
                "{unit} has no {name}.service"
            );
        } else if unit.ends_with(".service") {
            let text = std::fs::read_to_string(dir.join(unit)).unwrap();
            let script = &exec_start
                .captures(&text)
                .unwrap_or_else(|| panic!("{unit} starts nothing in /opt/bookclub"))[1];
            let mode = std::fs::metadata(repo().join("deploy/hosted").join(script))
                .unwrap_or_else(|error| panic!("{unit} starts {script}: {error}"))
                .permissions()
                .mode();
            assert!(mode & 0o111 != 0, "{script} is not executable");
            scripts.insert(script.to_owned());
        }
    }
    for script in ["backup.sh", "restore-test.sh", "update.sh"] {
        assert!(
            scripts.contains(script),
            "no unit starts {script}: {units:?}"
        );
    }
}

/// A stand-in VM around copies of `update.sh` and `backup-lib.sh`:
/// - `docker` and `curl` are bash functions, imported from the environment, that log each call
///   to `calls.log` and answer from the files in `state/`: `tag` is the image the followed tag
///   names after a pull, `running` the image the app container runs, `status` its state and
///   health, `after-up` the status a `compose up` leaves, `registry-down` makes the pull fail;
/// - `backup.sh` is a stub that logs its run and exits with `state/backup-status` (default 0).
struct UpdateRig {
    dir: tempfile::TempDir,
}

const FAKE_DOCKER: &str = r#"() {
    printf 'docker %s\n' "$*" >>"$RIG/calls.log"
    case "$*" in
        "compose config --images app") printf '%s\n' postgres:17 registry.test/bookclub-server:stable ;;
        "compose ps -a -q app") echo app-container ;;
        *com.docker.compose.oneoff*) echo False ;;
        *State.Status*) cat "$RIG/state/status" ;;
        "compose pull --quiet app") [ ! -e "$RIG/state/registry-down" ] ;;
        "image inspect -f {{.Id}} "*) cat "$RIG/state/tag" ;;
        "inspect -f {{.Image}} app-container") cat "$RIG/state/running" ;;
        "compose up "*)
            cp "$RIG/state/tag" "$RIG/state/running"
            cp "$RIG/state/after-up" "$RIG/state/status"
            [ "$(cat "$RIG/state/status")" = "running healthy" ] ;;
        *RepoTags*) echo ;;
        *RepoDigests*) echo "registry.test/bookclub-server@${*: -1}" ;;
        "image rm "*) ;;
        *) echo "unexpected: docker $*" >&2; return 99 ;;
    esac
}"#;

impl UpdateRig {
    /// The app runs `sha256:a`, healthy, and the tag names `tag`.
    fn new(tag: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let opt = root.join("opt");
        std::fs::create_dir_all(&opt).unwrap();
        std::fs::create_dir_all(root.join("state")).unwrap();
        for script in ["update.sh", "backup-lib.sh"] {
            std::fs::copy(repo().join("deploy/hosted").join(script), opt.join(script)).unwrap();
        }
        std::fs::write(
            opt.join("backup.sh"),
            "#!/usr/bin/env bash\nprintf 'backup.sh\\n' >>\"$RIG/calls.log\"\n\
             exit \"$(cat \"$RIG/state/backup-status\" 2>/dev/null || echo 0)\"\n",
        )
        .unwrap();
        std::fs::set_permissions(
            opt.join("backup.sh"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        std::fs::write(opt.join("restic-password"), "x\n").unwrap();
        let root_str = root.display();
        std::fs::write(
            opt.join("backup.env"),
            format!(
                "COMPOSE_DIR={root_str}/opt\nMEDIA_DIR={root_str}/media\n\
                 BACKUP_DIR={root_str}/backup\nRESTIC_REPOSITORY={root_str}/repo\n\
                 RESTIC_PASSWORD_FILE={root_str}/opt/restic-password\n\
                 UPDATE_HEARTBEAT_URL=https://hc.test/ping/update\n"
            ),
        )
        .unwrap();
        let rig = Self { dir };
        rig.set("tag", tag);
        rig.set("running", "sha256:a");
        rig.set("status", "running healthy");
        rig.set("after-up", "running healthy");
        rig
    }

    fn set(&self, name: &str, value: &str) {
        std::fs::write(
            self.dir.path().join("state").join(name),
            format!("{value}\n"),
        )
        .unwrap();
    }

    fn get(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.path().join("state").join(name))
            .unwrap()
            .trim()
            .to_owned()
    }

    /// Runs `update.sh` once; returns its output and the calls it made, in order.
    fn run(&self) -> (Output, Vec<String>) {
        let log = self.dir.path().join("calls.log");
        let _ = std::fs::remove_file(&log);
        let output = std::process::Command::new("bash")
            .arg(self.dir.path().join("opt/update.sh"))
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("RIG", self.dir.path())
            .env("BASH_FUNC_docker%%", FAKE_DOCKER)
            .env(
                "BASH_FUNC_curl%%",
                r#"() { printf 'ping %s\n' "${*: -1}" >>"$RIG/calls.log"; }"#,
            )
            .output()
            .unwrap();
        let calls = std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect();
        (output, calls)
    }
}

/// What a script printed: stdout, then stderr. `update.sh` logs an update's errors on stdout too.
fn printed(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        stderr(output)
    )
}

fn pings(calls: &[String]) -> Vec<&str> {
    calls
        .iter()
        .filter_map(|call| call.strip_prefix("ping https://hc.test/ping/update/"))
        .collect()
}

fn position(calls: &[String], prefix: &str) -> usize {
    calls
        .iter()
        .position(|call| call.starts_with(prefix))
        .unwrap_or_else(|| panic!("no call {prefix:?} in {calls:#?}"))
}

/// While the tag names the image the app runs, a run pulls, backs nothing up, replaces nothing,
/// and pings 0; a starting app gets no ping.
#[cfg(unix)]
#[test]
fn update_leaves_an_up_to_date_app_alone() {
    let rig = UpdateRig::new("sha256:a");
    let (output, calls) = rig.run();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(calls.contains(&"docker compose pull --quiet app".to_owned()));
    assert!(
        !calls
            .iter()
            .any(|call| call == "backup.sh" || call.starts_with("docker compose up"))
    );
    assert_eq!(pings(&calls), ["0"], "{calls:#?}");

    rig.set("status", "running starting");
    let (output, calls) = rig.run();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(pings(&calls).is_empty(), "{calls:#?}");
}

/// When the tag has moved: /start, the backup, then the new container, then 0; the image the
/// app ran before is removed once no tag names it.
#[cfg(unix)]
#[test]
fn update_backs_up_before_it_replaces_the_app() {
    let rig = UpdateRig::new("sha256:b");
    let (output, calls) = rig.run();
    assert!(output.status.success(), "{}", stderr(&output));
    let start = position(&calls, "ping https://hc.test/ping/update/start");
    let backup = position(&calls, "backup.sh");
    let up = position(
        &calls,
        "docker compose up -d --no-deps --wait --wait-timeout 300 app",
    );
    let done = position(&calls, "ping https://hc.test/ping/update/0");
    assert!(start < backup && backup < up && up < done, "{calls:#?}");
    assert_eq!(rig.get("running"), "sha256:b");
    assert!(
        calls.contains(&"docker image rm sha256:a".to_owned()),
        "{calls:#?}"
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("and is healthy"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// A new image that does not turn healthy fails the run and pings 1, and so does every later run
/// while the app stays unhealthy on it, without another backup.
#[cfg(unix)]
#[test]
fn update_reports_an_app_that_does_not_turn_healthy() {
    let rig = UpdateRig::new("sha256:b");
    rig.set("after-up", "running unhealthy");
    let (output, calls) = rig.run();
    assert!(!output.status.success());
    assert!(position(&calls, "backup.sh") < position(&calls, "docker compose up"));
    assert_eq!(pings(&calls), ["start", "1"], "{calls:#?}");
    assert!(!calls.iter().any(|call| call.starts_with("docker image rm")));
    assert!(
        printed(&output).contains("did not turn healthy"),
        "{}",
        printed(&output)
    );

    let (output, calls) = rig.run();
    assert!(!output.status.success());
    assert!(!calls.iter().any(|call| call == "backup.sh"), "{calls:#?}");
    assert_eq!(pings(&calls), ["1"], "{calls:#?}");

    rig.set("status", "restarting unhealthy");
    let (output, calls) = rig.run();
    assert!(!output.status.success());
    assert_eq!(pings(&calls), ["1"], "{calls:#?}");
}

/// A failed backup leaves the app as it was and pings 1; the next run tries again.
#[cfg(unix)]
#[test]
fn update_never_replaces_the_app_without_a_backup() {
    let rig = UpdateRig::new("sha256:b");
    rig.set("backup-status", "1");
    let (output, calls) = rig.run();
    assert!(!output.status.success());
    assert!(calls.contains(&"backup.sh".to_owned()));
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("docker compose up")),
        "{calls:#?}"
    );
    assert_eq!(pings(&calls), ["start", "1"], "{calls:#?}");
    assert_eq!(rig.get("running"), "sha256:a");

    rig.set("backup-status", "0");
    let (output, calls) = rig.run();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(pings(&calls), ["start", "0"], "{calls:#?}");
    assert_eq!(rig.get("running"), "sha256:b");
}

/// A stopped app (the import, a restore) is never pulled for, started or pinged about, and a
/// registry that does not answer fails the run without a ping and without a backup.
#[cfg(unix)]
#[test]
fn update_leaves_a_stopped_app_and_a_dead_registry_alone() {
    let rig = UpdateRig::new("sha256:b");
    rig.set("status", "exited none");
    let (output, calls) = rig.run();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("docker compose pull")
                || call.starts_with("docker compose up")
                || call.starts_with("ping")),
        "{calls:#?}"
    );
    assert_eq!(rig.get("running"), "sha256:a");

    rig.set("status", "running healthy");
    rig.set("registry-down", "");
    let (output, calls) = rig.run();
    assert!(!output.status.success());
    assert!(
        printed(&output).contains("cannot pull"),
        "{}",
        printed(&output)
    );
    assert!(
        !calls
            .iter()
            .any(|call| call == "backup.sh" || call.starts_with("ping")),
        "{calls:#?}"
    );
    assert_eq!(rig.get("running"), "sha256:a");
}
