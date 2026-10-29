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
//!   replaces the app, never touches an app that is up to date or stopped (also when it was
//!   stopped during the backup), waits an hour before it tries a failed image again, runs once at
//!   a time, ignores one-off containers, and pings the heartbeat with the outcome without putting
//!   the ping URL on a command line or into the log.
//! - The heartbeat pings of `update.sh` and of `supervise` (`backup-lib.sh`, which runs
//!   `backup.sh` and `restore-test.sh`) carry the end of the log without any picture's public id.
//! - `deploy/hosted/backup.sh`, with `docker` stood in for, keeps every snapshot of the last 14
//!   days, so the backup before an update survives later backups the same day.

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
///   to `calls.log` and answer from the files in `state/`: `ps` the IDs `compose ps -a -q app`
///   prints (`app-container`, the service's container, and `oneoff-1`, a running one-off
///   container of `docker compose run app`), `tag` the image the followed tag names after a pull,
///   `running` the image `app-container` runs, `status` its state and health, `after-up` the
///   status a `compose up` leaves, `up-exit` (optional) `compose up`'s exit status, else 0 only
///   for `running healthy`, `old-tags` what `RepoTags` prints for the old image, `registry-down`
///   makes the pull fail and `ping-down` every ping. `inspect` of any other ID fails as Docker's
///   does (`No such object`).
/// - `curl` reads the ping URL from its config on stdin (`-K -`) and fails when the URL is on its
///   command line. It appends the body of each ping (`--data-binary @<file>`) to `bodies.log`.
/// - `backup.sh` is a stub that logs its run, prints `state/backup-says` (if there is one) as
///   its output, copies the files in `state/after-backup/` over `state/` (whatever happened while
///   it ran) and exits with `state/backup-status` (default 0).
struct UpdateRig {
    dir: tempfile::TempDir,
}

const FAKE_DOCKER: &str = r#"() {
    printf 'docker %s\n' "$*" >>"$RIG/calls.log"
    local id=${*: -1}
    case "$*" in
        "compose config --images app") printf '%s\n' postgres:17 registry.test/bookclub-server:stable ;;
        "compose ps -a -q app") cat "$RIG/state/ps" ;;
        "compose pull --quiet app") [ ! -e "$RIG/state/registry-down" ] ;;
        "compose up "*)
            cp "$RIG/state/tag" "$RIG/state/running"
            cp "$RIG/state/after-up" "$RIG/state/status"
            if [ -e "$RIG/state/up-exit" ]; then return "$(cat "$RIG/state/up-exit")"; fi
            [ "$(cat "$RIG/state/status")" = "running healthy" ] ;;
        "image inspect -f {{.Id}} "*) cat "$RIG/state/tag" ;;
        *RepoTags*) cat "$RIG/state/old-tags" 2>/dev/null || echo ;;
        *RepoDigests*) echo "registry.test/bookclub-server@$id" ;;
        "image rm "*) ;;
        "inspect "*)
            case $id in
                app-container | oneoff-1) ;;
                *) echo "Error: No such object: $id" >&2; return 1 ;;
            esac
            case "$*:$id" in
                *com.docker.compose.oneoff*:oneoff-1) echo True ;;
                *com.docker.compose.oneoff*) echo False ;;
                *State.Status*:oneoff-1) echo "running none" ;;
                *State.Status*) cat "$RIG/state/status" ;;
                "inspect -f {{.Image}} app-container:app-container") cat "$RIG/state/running" ;;
                *) echo "unexpected: docker $*" >&2; return 99 ;;
            esac ;;
        *) echo "unexpected: docker $*" >&2; return 99 ;;
    esac
}"#;

const FAKE_CURL: &str = r#"() {
    case "$*" in *hc.test*) echo "curl: the ping URL is on the command line: $*" >&2; return 99 ;; esac
    if [ -e "$RIG/state/ping-down" ]; then
        cat >/dev/null
        echo "curl: (7) Failed to connect" >&2
        return 7
    fi
    local previous='' arg
    for arg in "$@"; do
        if [ "$previous" = --data-binary ]; then cat "${arg#@}" >>"$RIG/bodies.log"; fi
        previous=$arg
    done
    sed -n 's/^url = "\(.*\)"$/ping \1/p' >>"$RIG/calls.log"
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
             if [ -r \"$RIG/state/backup-says\" ]; then cat \"$RIG/state/backup-says\"; fi\n\
             if [ -d \"$RIG/state/after-backup\" ]; then cp \"$RIG/state/after-backup/\"* \"$RIG/state/\"; fi\n\
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
        rig.set("ps", "app-container");
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

    /// What the stub `backup.sh` sets while it runs.
    fn set_after_backup(&self, name: &str, value: &str) {
        let dir = self.dir.path().join("state/after-backup");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), format!("{value}\n")).unwrap();
    }

    fn get(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.path().join("state").join(name))
            .unwrap()
            .trim()
            .to_owned()
    }

    /// `BACKUP_DIR`, which `update.sh` creates on its first run.
    fn backup_dir(&self) -> PathBuf {
        self.dir.path().join("backup")
    }

    /// Runs `update.sh` once; returns its output and the calls it made, in order.
    fn run(&self) -> (Output, Vec<String>) {
        self.run_with(&[])
    }

    /// Runs `update.sh` once under the command `prefix` (for example `flock <file>`).
    fn run_with(&self, prefix: &[&std::ffi::OsStr]) -> (Output, Vec<String>) {
        let log = self.dir.path().join("calls.log");
        let _ = std::fs::remove_file(&log);
        let _ = std::fs::remove_file(self.dir.path().join("bodies.log"));
        let script = self.dir.path().join("opt/update.sh");
        let mut argv: Vec<&std::ffi::OsStr> = prefix.to_vec();
        argv.push("bash".as_ref());
        argv.push(script.as_os_str());
        let output = std::process::Command::new(argv[0])
            .args(&argv[1..])
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("RIG", self.dir.path())
            .env("BASH_FUNC_docker%%", FAKE_DOCKER)
            .env("BASH_FUNC_curl%%", FAKE_CURL)
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

/// The bodies of the last run's pings, in order.
fn bodies(rig: &Path) -> String {
    std::fs::read_to_string(rig.join("bodies.log")).unwrap_or_default()
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

fn has_call(calls: &[String], prefix: &str) -> bool {
    calls.iter().any(|call| call.starts_with(prefix))
}

/// While the tag names the image the app runs, a run pulls, backs nothing up, replaces nothing,
/// and pings 0; a starting app gets no ping. A failed ping is only logged, and the log names the
/// host but not the URL, whose path is the check's secret.
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

    rig.set("ping-down", "");
    let (output, _) = rig.run();
    assert!(output.status.success(), "{}", stderr(&output));
    let text = printed(&output);
    assert!(
        text.contains("warning: the heartbeat ping 0 to hc.test failed"),
        "{text}"
    );
    assert!(!text.contains("/ping/update"), "{text}");
    std::fs::remove_file(rig.dir.path().join("state/ping-down")).unwrap();

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

/// An image that a tag still names (an older release, or a pin) is kept for a rollback.
#[cfg(unix)]
#[test]
fn update_keeps_an_old_image_that_a_tag_still_names() {
    let rig = UpdateRig::new("sha256:b");
    rig.set("old-tags", "registry.test/bookclub-server:0.11.0");
    let (output, calls) = rig.run();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(pings(&calls), ["start", "0"], "{calls:#?}");
    assert_eq!(rig.get("running"), "sha256:b");
    assert!(!has_call(&calls, "docker image rm"), "{calls:#?}");
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
    assert!(!has_call(&calls, "docker image rm"));
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

/// `compose up` that succeeds is not trusted alone: an app that is not healthy afterwards fails
/// the update.
#[cfg(unix)]
#[test]
fn update_checks_the_app_after_compose_up() {
    let rig = UpdateRig::new("sha256:b");
    rig.set("up-exit", "0");
    rig.set("after-up", "running unhealthy");
    let (output, calls) = rig.run();
    assert!(!output.status.success());
    assert_eq!(pings(&calls), ["start", "1"], "{calls:#?}");
    assert!(!has_call(&calls, "docker image rm"), "{calls:#?}");
    assert!(
        printed(&output).contains("the app is running unhealthy after the update"),
        "{}",
        printed(&output)
    );
}

/// A failed backup leaves the app as it was and pings 1. The same image is tried again only an
/// hour later, because every try starts with a full backup; the runs in between ping 1 without
/// one.
#[cfg(unix)]
#[test]
fn update_never_replaces_the_app_without_a_backup() {
    let rig = UpdateRig::new("sha256:b");
    rig.set("backup-status", "1");
    let (output, calls) = rig.run();
    assert!(!output.status.success());
    assert!(calls.contains(&"backup.sh".to_owned()));
    assert!(!has_call(&calls, "docker compose up"), "{calls:#?}");
    assert_eq!(pings(&calls), ["start", "1"], "{calls:#?}");
    assert_eq!(rig.get("running"), "sha256:a");

    rig.set("backup-status", "0");
    let (output, calls) = rig.run();
    assert!(!output.status.success());
    assert!(!calls.iter().any(|call| call == "backup.sh"), "{calls:#?}");
    assert!(!has_call(&calls, "docker compose up"), "{calls:#?}");
    assert_eq!(pings(&calls), ["1"], "{calls:#?}");
    assert!(
        printed(&output).contains("the next try comes after"),
        "{}",
        printed(&output)
    );
    assert_eq!(rig.get("running"), "sha256:a");

    // An hour later.
    let failed = rig.backup_dir().join("update.failed");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(&failed, format!("sha256:b {}\n", now - 3601)).unwrap();
    let (output, calls) = rig.run();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(pings(&calls), ["start", "0"], "{calls:#?}");
    assert_eq!(rig.get("running"), "sha256:b");
    assert!(!failed.exists());
}

/// The wait after a failed update applies to that image only: a new image for the tag, or the
/// operator's `rm BACKUP_DIR/update.failed`, is tried at once.
#[cfg(unix)]
#[test]
fn update_tries_a_new_image_at_once() {
    let rig = UpdateRig::new("sha256:b");
    rig.set("backup-status", "1");
    let (output, _) = rig.run();
    assert!(!output.status.success());
    rig.set("backup-status", "0");

    rig.set("tag", "sha256:c");
    let (output, calls) = rig.run();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(pings(&calls), ["start", "0"], "{calls:#?}");
    assert_eq!(rig.get("running"), "sha256:c");

    let rig = UpdateRig::new("sha256:b");
    rig.set("backup-status", "1");
    let (output, _) = rig.run();
    assert!(!output.status.success());
    rig.set("backup-status", "0");
    std::fs::remove_file(rig.backup_dir().join("update.failed")).unwrap();
    let (output, calls) = rig.run();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(pings(&calls), ["start", "0"], "{calls:#?}");
    assert_eq!(rig.get("running"), "sha256:b");
}

/// An app stopped while the update's backup ran (`docker compose stop app` for the import, or
/// `down` for a restore, after the timer was stopped but before the run ended) stays stopped:
/// no `compose up`, and the update fails with ping 1. Once the app runs again, the update
/// follows at once.
#[cfg(unix)]
#[test]
fn update_never_starts_an_app_stopped_during_its_backup() {
    for (name, value) in [("status", "exited none"), ("ps", "")] {
        let rig = UpdateRig::new("sha256:b");
        rig.set_after_backup(name, value);
        let (output, calls) = rig.run();
        assert!(!output.status.success(), "{name}");
        assert!(calls.contains(&"backup.sh".to_owned()), "{calls:#?}");
        assert!(!has_call(&calls, "docker compose up"), "{calls:#?}");
        assert_eq!(pings(&calls), ["start", "1"], "{calls:#?}");
        assert!(
            printed(&output).contains("it was stopped while the backup ran"),
            "{}",
            printed(&output)
        );
        assert_eq!(rig.get("running"), "sha256:a");

        std::fs::remove_dir_all(rig.dir.path().join("state/after-backup")).unwrap();
        let (output, calls) = rig.run();
        assert!(output.status.success(), "{}", stderr(&output));
        assert!(
            !calls.iter().any(|call| call == "backup.sh"
                || call.starts_with("docker compose pull")
                || call.starts_with("ping")),
            "{calls:#?}"
        );

        rig.set("ps", "app-container");
        rig.set("status", "running healthy");
        let (output, calls) = rig.run();
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(pings(&calls), ["start", "0"], "{calls:#?}");
        assert_eq!(rig.get("running"), "sha256:b");
    }
}

/// A stopped app (the import, a restore) and a stack without an app container are never pulled
/// for, started or pinged about, and a registry that does not answer fails the run without a
/// ping and without a backup.
#[cfg(unix)]
#[test]
fn update_leaves_a_stopped_app_and_a_dead_registry_alone() {
    let untouched = |calls: &[String]| {
        !calls.iter().any(|call| {
            call == "backup.sh"
                || call.starts_with("docker compose pull")
                || call.starts_with("docker compose up")
                || call.starts_with("ping")
        })
    };
    let rig = UpdateRig::new("sha256:b");
    rig.set("status", "exited none");
    let (output, calls) = rig.run();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(untouched(&calls), "{calls:#?}");
    assert_eq!(rig.get("running"), "sha256:a");

    rig.set("ps", "");
    let (output, calls) = rig.run();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(untouched(&calls), "{calls:#?}");
    assert!(
        printed(&output).contains("there is no app container"),
        "{}",
        printed(&output)
    );

    rig.set("ps", "app-container");
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

/// The one-off containers of `docker compose run app` (the import, `rematch-legacy`) carry the
/// service's name but are not the app: only the service's own container is inspected, and a
/// running one-off container next to a stopped app does not make the app count as running.
#[cfg(unix)]
#[test]
fn update_ignores_one_off_containers() {
    let only_label_lookups = |calls: &[String]| {
        calls
            .iter()
            .filter(|call| call.contains("oneoff-1"))
            .all(|call| call.contains("com.docker.compose.oneoff"))
    };
    let rig = UpdateRig::new("sha256:a");
    rig.set("ps", "oneoff-1\napp-container");
    let (output, calls) = rig.run();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(pings(&calls), ["0"], "{calls:#?}");
    assert!(only_label_lookups(&calls), "{calls:#?}");

    rig.set("tag", "sha256:b");
    let (output, calls) = rig.run();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(pings(&calls), ["start", "0"], "{calls:#?}");
    assert_eq!(rig.get("running"), "sha256:b");
    assert!(only_label_lookups(&calls), "{calls:#?}");

    rig.set("status", "exited none");
    rig.set("tag", "sha256:c");
    let (output, calls) = rig.run();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        !calls
            .iter()
            .any(|call| call.starts_with("docker compose pull") || call.starts_with("ping")),
        "{calls:#?}"
    );
    assert_eq!(rig.get("running"), "sha256:b");
}

/// While another run holds the lock, a run changes nothing and makes no call.
#[cfg(unix)]
#[test]
fn update_runs_once_at_a_time() {
    let rig = UpdateRig::new("sha256:b");
    std::fs::create_dir_all(rig.backup_dir()).unwrap();
    let lock = rig.backup_dir().join(".update.lock");
    let (output, calls) = rig.run_with(&["flock".as_ref(), lock.as_os_str()]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        printed(&output).contains("another update.sh is running"),
        "{}",
        printed(&output)
    );
    assert!(calls.is_empty(), "{calls:#?}");
    assert_eq!(rig.get("running"), "sha256:a");
}

/// `UPDATE_WAIT_SECONDS` is checked by `update.sh` alone: a bad value stops the updates, before
/// any pull or ping, but `load_config` (and so `backup.sh` and `restore-test.sh`) still runs.
#[cfg(unix)]
#[test]
fn a_bad_update_setting_stops_only_the_updates() {
    let rig = UpdateRig::new("sha256:b");
    let env = rig.dir.path().join("opt/backup.env");
    let mut text = std::fs::read_to_string(&env).unwrap();
    text.push_str("UPDATE_WAIT_SECONDS=5m\n");
    std::fs::write(&env, text).unwrap();

    let (output, calls) = rig.run();
    assert!(!output.status.success());
    assert!(
        printed(&output).contains("UPDATE_WAIT_SECONDS=5m is not a number of seconds"),
        "{}",
        printed(&output)
    );
    assert!(calls.is_empty(), "{calls:#?}");

    let output = std::process::Command::new("bash")
        .args([
            "-c",
            r#"set -euo pipefail; here=$1; . "$here/backup-lib.sh"; load_config; echo "loaded $BACKUP_DIR""#,
            "bash",
        ])
        .arg(rig.dir.path().join("opt"))
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", printed(&output));
    assert!(
        String::from_utf8_lossy(&output.stdout).starts_with("loaded "),
        "{}",
        printed(&output)
    );
}

/// A picture's public id: 26 characters of `[a-z0-9]` (`media.public_id`'s check), the secret
/// part of its URL.
const PUBLIC_ID: &str = "k7m2q9x4p1r8t5w3z6b0c2d4f6";

/// The update's ping carries the end of its log, which holds `backup.sh`'s output. A line there
/// can name a picture by its path (one deleted during the backup), so the ping has `<id>` in
/// place of every public id, and healthchecks.io receives none (`docs/legal/processors.md`). The
/// log on the VM keeps it.
#[cfg(unix)]
#[test]
fn update_pings_carry_no_public_id() {
    let rig = UpdateRig::new("sha256:b");
    rig.set(
        "backup-says",
        &format!("warning: avatars/k7/{PUBLIC_ID}/a was deleted during the backup"),
    );
    let (output, calls) = rig.run();
    assert!(output.status.success(), "{}", printed(&output));
    assert_eq!(pings(&calls), ["start", "0"], "{calls:#?}");
    let body = bodies(rig.dir.path());
    assert!(
        body.contains("warning: avatars/k7/<id>/a was deleted during the backup"),
        "{body}"
    );
    assert!(!body.contains(PUBLIC_ID), "{body}");
    let log = std::fs::read_to_string(rig.backup_dir().join("update.log")).unwrap();
    assert!(log.contains(PUBLIC_ID), "{log}");
}

/// `supervise` (`backup-lib.sh`), which runs `backup.sh` and `restore-test.sh`, blanks the
/// public ids in its ping the same way, and pings the run's exit status.
#[cfg(unix)]
#[test]
fn supervise_pings_carry_no_public_id() {
    const SCRIPT: &str = r#"set -euo pipefail
. "$1"
BACKUP_DIR=$RIG/backup
said() {
    printf 'restic: avatars/k7/%s/a: no such file or directory\n' "$1"
    return 3
}
status=0
supervise backup https://hc.test/ping/backup said "$2" || status=$?
exit "$status"
"#;
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new("bash")
        .args(["-c", SCRIPT, "supervise"])
        .arg(repo().join("deploy/hosted/backup-lib.sh"))
        .arg(PUBLIC_ID)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("RIG", dir.path())
        .env("BASH_FUNC_curl%%", FAKE_CURL)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{}", printed(&output));
    let calls = std::fs::read_to_string(dir.path().join("calls.log")).unwrap_or_default();
    assert_eq!(
        calls.lines().collect::<Vec<_>>(),
        [
            "ping https://hc.test/ping/backup/start",
            "ping https://hc.test/ping/backup/3"
        ],
        "{}",
        printed(&output)
    );
    let body = bodies(dir.path());
    assert!(
        body.contains("restic: avatars/k7/<id>/a: no such file or directory"),
        "{body}"
    );
    assert!(!body.contains(PUBLIC_ID), "{body}");
    let log = std::fs::read_to_string(dir.path().join("backup/backup.log")).unwrap();
    assert!(log.contains(PUBLIC_ID), "{log}");
}

/// A fictional row as Postgres quotes it in an error: the data a ping must not carry.
const ROW_ADDRESS: &str = "alice@example.org";

/// `supervise` withholds the lines Postgres adds to an error (DETAIL, CONTEXT, …): they quote
/// the data, here a duplicated address and a failing row with its password hash (security
/// review, ADM2 merge). The error line itself, which names the index, reaches the ping; the log
/// on the VM keeps everything.
#[cfg(unix)]
#[test]
fn supervise_pings_withhold_postgres_detail() {
    const SCRIPT: &str = r#"set -euo pipefail
. "$1"
BACKUP_DIR=$RIG/backup
failed() {
    printf 'pg_restore: error: could not execute query: ERROR:  could not create unique index "users_verified_email_key"\n'
    printf 'DETAIL:  Key (lower(email))=(%s) is duplicated.\n' "$1"
    printf 'CONTEXT:  COPY users, line 1: "1\talice\t%s\t$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHQ"\n' "$1"
    printf 'HINT:  %s\n' "$1"
    printf 'Command was: INSERT INTO users VALUES (1, %s);\n' "$1"
    return 1
}
status=0
supervise restore-test https://hc.test/ping/restore failed "$2" || status=$?
exit "$status"
"#;
    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new("bash")
        .args(["-c", SCRIPT, "supervise"])
        .arg(repo().join("deploy/hosted/backup-lib.sh"))
        .arg(ROW_ADDRESS)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("RIG", dir.path())
        .env("BASH_FUNC_curl%%", FAKE_CURL)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", printed(&output));
    let body = bodies(dir.path());
    assert!(
        body.contains(
            "pg_restore: error: could not execute query: ERROR:  could not create unique index \
             \"users_verified_email_key\""
        ),
        "{body}"
    );
    for line in ["DETAIL", "CONTEXT", "HINT", "Command was"] {
        assert!(
            body.contains(&format!("{line}: <withheld>")),
            "{line}: {body}"
        );
    }
    assert!(!body.contains(ROW_ADDRESS), "{body}");
    assert!(!body.contains("argon2id"), "{body}");
    let log = std::fs::read_to_string(dir.path().join("backup/restore-test.log")).unwrap();
    assert!(log.contains(ROW_ADDRESS), "{log}");
}

/// `restore-test.sh` against stand-ins for `docker` and `curl`, with a restore that fails the way
/// a corrupted unique index in production makes it fail. Its ping names the index and where the
/// full messages are, and carries no address; the messages stay on the VM.
#[cfg(unix)]
#[test]
fn a_failed_restore_test_pings_no_row() {
    const FAKE_DOCKER: &str = r#"() {
        printf 'docker %s\n' "$*" >>"$RIG/calls.log"
        case "$*" in
            "run --rm "*" restore latest "*)
                local all="$*" target
                target=${all##*--target }
                target=${target%% *}
                mkdir -p "$target$RIG/backup/db" "$target$RIG/media"
                printf 'PGDMP' >"$target$RIG/backup/db/bookclub.dump"
                printf 'users\t1\n' >"$target$RIG/backup/db/counts.tsv" ;;
            "run -d --name bookclub-restore-test "*) ;;
            "exec bookclub-restore-test pg_isready "*) ;;
            "exec bookclub-restore-test pg_restore "*)
                printf 'pg_restore: error: could not execute query: ERROR:  could not create unique index "users_verified_email_key"\n' >&2
                printf 'DETAIL:  Key (lower(email))=(alice@example.org) is duplicated.\n' >&2
                printf 'pg_restore: error: COPY failed for table "users": ERROR:  invalid input syntax for type uuid: "alice@example.org"\n' >&2
                printf 'CONTEXT:  COPY users, line 1: "1\talice\talice@example.org"\n' >&2
                return 1 ;;
            "rm -fv bookclub-restore-test") ;;
            *) echo "unexpected: docker $*" >&2; return 99 ;;
        esac
    }"#;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let opt = root.join("opt");
    std::fs::create_dir_all(&opt).unwrap();
    for script in ["restore-test.sh", "backup-lib.sh"] {
        std::fs::copy(repo().join("deploy/hosted").join(script), opt.join(script)).unwrap();
    }
    std::fs::write(opt.join("restic-password"), "x\n").unwrap();
    let root_str = root.display();
    std::fs::write(
        opt.join("backup.env"),
        format!(
            "COMPOSE_DIR={root_str}/opt\nMEDIA_DIR={root_str}/media\n\
             BACKUP_DIR={root_str}/backup\nRESTIC_REPOSITORY={root_str}/repo\n\
             RESTIC_PASSWORD_FILE={root_str}/opt/restic-password\n\
             RESTORE_TEST_HEARTBEAT_URL=https://hc.test/ping/restore\n"
        ),
    )
    .unwrap();
    let output = std::process::Command::new("bash")
        .arg(opt.join("restore-test.sh"))
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("RIG", root)
        .env("BASH_FUNC_docker%%", FAKE_DOCKER)
        .env("BASH_FUNC_curl%%", FAKE_CURL)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", printed(&output));
    let calls = std::fs::read_to_string(root.join("calls.log")).unwrap_or_default();
    assert!(
        calls.contains("ping https://hc.test/ping/restore/1"),
        "{calls}"
    );
    assert!(
        calls.contains("docker rm -fv bookclub-restore-test"),
        "{calls}"
    );
    let body = bodies(root);
    assert!(
        body.contains(
            "pg_restore: error: could not execute query: ERROR:  could not create unique index \
             \"users_verified_email_key\""
        ),
        "{body}"
    );
    assert!(
        body.contains(
            "pg_restore: error: COPY failed for table \"users\": ERROR:  invalid input syntax for \
             type uuid: <withheld>"
        ),
        "{body}"
    );
    assert!(
        body.contains(&format!(
            "pg_restore failed; its full messages are in {root_str}/backup/pg_restore.err"
        )),
        "{body}"
    );
    assert!(!body.contains(ROW_ADDRESS), "{body}");
    let log = std::fs::read_to_string(root.join("backup/restore-test.log")).unwrap();
    assert!(!log.contains(ROW_ADDRESS), "{log}");
    let messages = std::fs::read_to_string(root.join("backup/pg_restore.err")).unwrap();
    assert!(messages.contains(ROW_ADDRESS), "{messages}");
}

/// `backup.sh` against a stand-in for `docker`, which logs every call to `calls.log` and answers
/// as an empty stack would (no app container, a dump and its counts, restic that succeeds).
/// Returns the output and the calls, in order.
#[cfg(unix)]
fn run_backup_sh() -> (Output, Vec<String>) {
    const FAKE_DOCKER: &str = r#"() {
        printf 'docker %s\n' "$*" >>"$RIG/calls.log"
        case "$*" in
            "compose ps -a -q app") ;;
            "compose exec -T postgres psql "*) cat >/dev/null; printf 'PGDMP' ;;
            "compose exec -T postgres cat "*) printf 'users\t1\n(migration)\t2201\n' ;;
            "compose exec -T postgres rm -f "*) ;;
            "compose exec -T postgres pg_restore --list") cat >/dev/null ;;
            "run "*) ;;
            *) echo "unexpected: docker $*" >&2; return 99 ;;
        esac
    }"#;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let opt = root.join("opt");
    std::fs::create_dir_all(&opt).unwrap();
    std::fs::create_dir_all(root.join("media")).unwrap();
    for script in ["backup.sh", "backup-lib.sh"] {
        std::fs::copy(repo().join("deploy/hosted").join(script), opt.join(script)).unwrap();
    }
    std::fs::write(opt.join("restic-password"), "x\n").unwrap();
    let root_str = root.display();
    std::fs::write(
        opt.join("backup.env"),
        format!(
            "COMPOSE_DIR={root_str}/opt\nMEDIA_DIR={root_str}/media\n\
             BACKUP_DIR={root_str}/backup\nRESTIC_REPOSITORY={root_str}/repo\n\
             RESTIC_PASSWORD_FILE={root_str}/opt/restic-password\n"
        ),
    )
    .unwrap();
    let output = std::process::Command::new("bash")
        .arg(opt.join("backup.sh"))
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("RIG", root)
        .env("BASH_FUNC_docker%%", FAKE_DOCKER)
        .output()
        .unwrap();
    let calls = std::fs::read_to_string(root.join("calls.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    (output, calls)
}

/// The backup `update.sh` takes before an update is the way back across a migration (README.md,
/// "Rollback"), so a later backup the same day must not prune it. restic's `--keep-daily` keeps
/// only the newest snapshot of each day: after an update at 10:00 and a hotfix at 15:00 (or the
/// nightly backup after an update past midnight), the snapshot from before the migration was gone
/// (security review, Phase 2 Wave 2). `backup.sh` therefore keeps every snapshot of the last 14
/// days, and the daily, weekly and monthly ones beyond them.
#[cfg(unix)]
#[test]
fn backups_keep_every_snapshot_of_the_last_two_weeks() {
    let (output, calls) = run_backup_sh();
    assert!(output.status.success(), "{}", printed(&output));
    let restic: Vec<&str> = calls
        .iter()
        .filter_map(|call| call.split_once(" --retry-lock 10m ").map(|(_, args)| args))
        .collect();
    assert!(
        restic
            .first()
            .is_some_and(|args| args.starts_with("backup ")),
        "{calls:#?}"
    );
    let forget = restic
        .iter()
        .find(|args| args.starts_with("forget "))
        .unwrap_or_else(|| panic!("no restic forget in {calls:#?}"));
    for keep in [
        "--keep-within 14d",
        "--keep-daily 7",
        "--keep-weekly 4",
        "--keep-monthly 12",
        "--prune",
    ] {
        assert!(forget.contains(keep), "{keep} missing in `{forget}`");
    }
}
