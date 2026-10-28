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
/// Named in `env.example` before the server reads them: LEG's `LEGAL_DIR` (#53). Once LEG's
/// `config.rs` reads it, the entry is redundant and may go.
const READ_BY_A_LATER_PACKAGE: &[&str] = &["LEGAL_DIR"];
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
        .filter(|name| {
            !server.contains(name)
                && !compose.contains(name)
                && !READ_BY_A_LATER_PACKAGE.contains(&name.as_str())
        })
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
