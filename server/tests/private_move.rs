//! The private move's runbook (MOVE #52, `docs/phase3/private-move.md`) where its blocks are code
//! that runs as root on the production host, and the layout note of
//! `deploy/cutover/host-facts.sh`. Each test reads the block from the runbook as it stands:
//! - `summary` (section 0) prints a report's lines before the first section only when it knows
//!   them, and each section as its title and a count: no line of a section reaches the terminal,
//!   also one that a title with a line break starts without indentation.
//! - The rollback's first step (section 8) counts the club's members that used the new app after
//!   "done", by a live session or by one that ended.
//! - The rollback's delete block stops before it touches anything when the move's variables are
//!   missing or `$NEW` is not the new stack.
//! - The M6 cleanup block (section 11) deletes the old data directory only when it holds nothing
//!   of the new stack or Docker's volumes, and only while the new app answers.
//! - `host-facts.sh` notes an old data directory that holds a path of the new layout.
//!
//! `bash`, `awk`, `find` and `realpath` are the real ones. `docker` and `curl` are bash
//! functions imported from the environment that log their calls.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use bookclub_server::importer::Report;
use sqlx::{AssertSqlSafe, PgPool};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}

fn runbook() -> String {
    std::fs::read_to_string(repo().join("docs/phase3/private-move.md")).unwrap()
}

/// The lines after the first line that ends with `<<'TAG'`, up to the line `TAG`, without the
/// indentation of that first line (the rollback's blocks sit in a list item).
fn heredoc(tag: &str) -> String {
    let text = runbook();
    let opener = format!("<<'{tag}'");
    let mut lines = text.lines();
    let start = lines
        .by_ref()
        .find(|line| line.trim_end().ends_with(&opener))
        .unwrap_or_else(|| panic!("no block {opener} in private-move.md"));
    let indent = start.len() - start.trim_start().len();
    let mut body = String::new();
    for line in lines {
        if line.trim() == tag {
            return body;
        }
        body.push_str(line.get(indent..).unwrap_or_else(|| line.trim_start()));
        body.push('\n');
    }
    panic!("the block {opener} in private-move.md has no end line {tag}");
}

/// `summary()` from section 0, as `vars.sh` defines it.
fn summary_function() -> String {
    let text = runbook();
    let start = text
        .find("\nsummary() {\n")
        .expect("no summary() in private-move.md");
    let end = start + text[start..].find("\n}\n").expect("summary() has no end") + 3;
    text[start..end].to_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// `bash -s` with `script` on stdin, in `dir`, with only `PATH` and `env` set.
fn bash_s(script: &str, dir: &Path, env: &[(&str, &std::ffi::OsStr)]) -> Output {
    use std::io::Write as _;

    let mut command = Command::new("bash");
    command
        .arg("-s")
        .current_dir(dir)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in env {
        command.env(name, value);
    }
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn calls(rig: &Path) -> Vec<String> {
    std::fs::read_to_string(rig.join("calls.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn write(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

// ---- summary --------------------------------------------------------------------------------

/// A report as `import-legacy` prints it (`importer/report.rs`), with titles that hold line
/// breaks: the part after each break starts a line without the section's indentation. Every
/// line that must stay on the host has `SECRET` in it.
const REPORT: &str = "\
users                  2 → 2
club                   - → 1 \"Bookclub\" (bookclub-8rqp), owner owner1, 2 members
books                  3 → 2 works (1 merged; isbn 1 matched)
push_subscriptions     1 → DROPPED (Phase 4)
normalised             books.pages 1
lookup wait            0.0 s
cover search           2 searched: 1 attached, 1 still without a cover (1 no match); 0 books readers added left as they are; 0 imported cover ids not fetched yet (checked once the server runs)
merges:
  book 10 → the work of book 9
isbn keys:
  /works/ISBN9780000000019 (book 10, isbn 9780000000019): matched \"First line
SECRET second line of a title:\"
without a cover:
  book 11 \"Another
SECRET: a title\" by Someone
SECRET author: no match
notes:
  isbn 9780000000019 (edition 4): upgraded from dnb: \"A title
SECRET tail\"
";

const SUMMARY: &str = "\
users                  2 → 2
club                   - → 1 \"Bookclub\" (bookclub-8rqp), owner owner1, 2 members
books                  3 → 2 works (1 merged; isbn 1 matched)
push_subscriptions     1 → DROPPED (Phase 4)
normalised             books.pages 1
lookup wait            0.0 s
cover search           2 searched: 1 attached, 1 still without a cover (1 no match); 0 books readers added left as they are; 0 imported cover ids not fetched yet (checked once the server runs)
merges: 1 lines
isbn keys: 2 lines
without a cover: 3 lines
notes: 2 lines
";

/// `summary` on a file (7.3) or on stdin (`rematch-legacy … | summary`, 7.5).
fn summary(report: &str, from_stdin: bool) -> String {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("report.txt");
    std::fs::write(&file, report).unwrap();
    let call = if from_stdin {
        "summary <report.txt"
    } else {
        "summary report.txt"
    };
    let output = bash_s(
        &format!("{}\n{call}\n", summary_function()),
        dir.path(),
        &[],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    stdout(&output)
}

/// The import's reports name books in their sections, and a title or an author may hold a line
/// break (an admin's edit keeps it): the text after it is a line of its own without the
/// section's indentation. `summary` prints none of a section's lines, whatever they look like,
/// and counts them; the lines before the first section it prints only when it knows them.
#[test]
fn summary_prints_no_line_of_a_section() {
    for from_stdin in [false, true] {
        let printed = summary(REPORT, from_stdin);
        assert!(!printed.contains("SECRET"), "{printed}");
        assert_eq!(printed, SUMMARY);
    }
    // A dry run's header, as `Report` prints it, passes; a line it does not know only counts.
    let dry = Report {
        dry_run: true,
        rematch: true,
        lines: vec![
            "legacy editions        3 → 3".to_owned(),
            "SECRET a line summary does not know".to_owned(),
            "lookup wait            0.0 s".to_owned(),
        ],
        ..Report::default()
    }
    .to_string();
    let printed = summary(&dry, false);
    let header: Vec<&str> = dry.lines().take(2).collect();
    assert_eq!(
        printed,
        format!(
            "{}\n{}\nlegacy editions        3 → 3\nlookup wait            0.0 s\nother lines 1\n",
            header[0], header[1]
        )
    );
}

// ---- the rollback ---------------------------------------------------------------------------

/// The block's count with `DONE_AT` as `done` (`infinity` before "done"), as `psql -v` puts it in.
async fn window(db: &PgPool, sql: &str, done: &str) -> i64 {
    assert!(sql.contains(":'done'"), "{sql}");
    let sql = sql.replace(":'done'", &format!("'{done}'::timestamptz"));
    sqlx::query_scalar(AssertSqlSafe(sql.clone()))
        .fetch_one(db)
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
}

async fn account(db: &PgPool, name: &str) -> i64 {
    sqlx::query_scalar("INSERT INTO users (username, password_hash) VALUES ($1, 'x') RETURNING id")
        .bind(name)
        .fetch_one(db)
        .await
        .unwrap()
}

/// A session of `user` last used at `used`.
async fn session(db: &PgPool, user: i64, token: &str, used: &str) {
    sqlx::query(
        "INSERT INTO sessions (user_id, kind, token_hash, expires_at, created_at, last_used_at)
         VALUES ($1, 'cookie', decode(repeat($2, 32), 'hex'), now() + interval '30 days',
                 $3::timestamptz, $3::timestamptz)",
    )
    .bind(user)
    .bind(token)
    .bind(used)
    .execute(db)
    .await
    .unwrap();
}

/// The rollback's first step counts the club's members, the owner included, that used the new
/// app after "done": a session used after `DONE_AT`, live or ended (a logout leaves
/// `last_seen_at`). Rafael's checks with the owner's account before "done" and his own admin
/// account, which is in no club, do not count; before "done" (`infinity`) nothing does.
#[sqlx::test]
async fn the_rollback_window_counts_members_after_done(db: PgPool) {
    const BEFORE: &str = "2026-11-10T10:00:00Z";
    const DONE: &str = "2026-11-10T11:00:00Z";
    const AFTER: &str = "2026-11-10T12:00:00Z";
    let sql = heredoc("SQL");
    assert_eq!(window(&db, &sql, "infinity").await, 0, "an empty target");

    let owner = account(&db, "owner1").await;
    let member = account(&db, "member1").await;
    // One transaction: the club's owner check is deferred to the commit.
    let mut tx = db.begin().await.unwrap();
    let club: i64 = sqlx::query_scalar(
        "INSERT INTO clubs (slug, name, created_by) VALUES ('bookclub-8rqp', 'Bookclub', $1)
         RETURNING id",
    )
    .bind(owner)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    for (user, role) in [(owner, "owner"), (member, "member")] {
        sqlx::query("INSERT INTO club_memberships (club_id, user_id, role) VALUES ($1, $2, $3)")
            .bind(club)
            .bind(user)
            .bind(role)
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
    assert_eq!(
        window(&db, &sql, DONE).await,
        0,
        "imported, nobody signed in"
    );

    session(&db, owner, "ab", BEFORE).await;
    assert_eq!(
        window(&db, &sql, "infinity").await,
        0,
        "Rafael's checks, before done"
    );
    assert_eq!(
        window(&db, &sql, DONE).await,
        0,
        "Rafael's checks, after done"
    );

    let admin = account(&db, "rafael").await;
    session(&db, admin, "ef", AFTER).await;
    assert_eq!(
        window(&db, &sql, DONE).await,
        0,
        "Rafael's own account, in no club"
    );

    session(&db, member, "cd", AFTER).await;
    assert_eq!(
        window(&db, &sql, DONE).await,
        1,
        "the other member signed in"
    );
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(member)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(window(&db, &sql, DONE).await, 1, "and signed out again");

    session(&db, owner, "01", AFTER).await;
    assert_eq!(
        window(&db, &sql, DONE).await,
        2,
        "the owner herself, after done"
    );
    assert_eq!(
        window(&db, &sql, "infinity").await,
        0,
        "before done, nothing counts"
    );
}

/// A host for the rollback's delete block: `new/` with `compose.yaml` and `.env`, `data/media`
/// with a picture, `data/import` with the copy, and `old/` with only a `compose.yaml` (the
/// Python app's directory).
struct RollbackRig {
    dir: tempfile::TempDir,
}

const LOGGING_DOCKER: &str = r#"() { printf 'docker %s\n' "$*" >>"$RIG/calls.log"; }"#;

impl RollbackRig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("new/compose.yaml"),
            "name: bookclub\nservices:\n  app:\n    image: git.tillhome.de/rafiistcool/bookclub-server:${BOOKCLUB_TAG:-stable}\n",
        );
        write(
            &root.join("new/.env"),
            &format!("BOOKCLUB_DATA_DIR={}/data\n", root.display()),
        );
        write(&root.join("data/media/covers/ab/c"), "picture");
        write(&root.join("data/import/bookclub.db"), "copy");
        write(&root.join("old/compose.yaml"), "services: {bookclub: {}}\n");
        let vars = format!(
            "NEW={0}/new\nDATA={0}/data\nIMPORT=$DATA/import\n",
            root.display()
        );
        let rig = Self { dir };
        rig.vars(&vars);
        rig
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn vars(&self, content: &str) {
        write(&self.root().join("vars.sh"), content);
    }

    /// Runs the block, with `vars.sh` read from the rig, from `old/` (as if a session had stayed
    /// in the directory of the step before).
    fn run(&self) -> (Output, Vec<String>) {
        let block = heredoc("ROLLBACK");
        assert!(block.contains(". /root/bookclub-move/vars.sh\n"), "{block}");
        let block = block.replace(
            "/root/bookclub-move/vars.sh",
            &self.root().join("vars.sh").display().to_string(),
        );
        let _ = std::fs::remove_file(self.root().join("calls.log"));
        let output = bash_s(
            &block,
            &self.root().join("old"),
            &[
                ("RIG", self.root().as_os_str()),
                ("BASH_FUNC_docker%%", LOGGING_DOCKER.as_ref()),
            ],
        );
        (output, calls(self.root()))
    }

    fn picture_kept(&self) -> bool {
        self.root().join("data/media/covers/ab/c").exists()
    }
}

/// A change to a [`RollbackRig`] that must stop the rollback's delete block.
type Breakage = fn(&RollbackRig);

/// The block removes the new stack with its volume and empties its media directory; the copy in
/// `$IMPORT` waits for step 6. It deletes nothing when `vars.sh` cannot be read or misses
/// `DATA`, when `$NEW` is not the new stack, or when the stack's `BOOKCLUB_DATA_DIR` is not
/// `$DATA`.
#[test]
fn the_rollback_deletes_only_the_new_stack() {
    let rig = RollbackRig::new();
    let (output, calls) = rig.run();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(calls, ["docker compose down -v"]);
    assert!(!rig.picture_kept());
    assert!(
        rig.root().join("data/media").is_dir(),
        "the directory stays"
    );
    assert!(rig.root().join("data/import/bookclub.db").exists());

    let broken: [(&str, Breakage); 5] = [
        ("no vars.sh", |rig| {
            std::fs::remove_file(rig.root().join("vars.sh")).unwrap()
        }),
        ("vars.sh without DATA", |rig| {
            rig.vars(&format!("NEW={}/new\n", rig.root().display()))
        }),
        ("NEW is the old app's directory", |rig| {
            rig.vars(&format!(
                "NEW={0}/old\nDATA={0}/data\n",
                rig.root().display()
            ))
        }),
        ("NEW holds another compose project", |rig| {
            write(
                &rig.root().join("new/compose.yaml"),
                "services: {bookclub: {image: git.tillhome.de/rafiistcool/bookclub:latest}}\n",
            )
        }),
        ("BOOKCLUB_DATA_DIR is not $DATA", |rig| {
            write(
                &rig.root().join("new/.env"),
                &format!("BOOKCLUB_DATA_DIR={}/elsewhere\n", rig.root().display()),
            )
        }),
    ];
    for (case, setup) in broken {
        let rig = RollbackRig::new();
        setup(&rig);
        let (output, calls) = rig.run();
        assert!(!output.status.success(), "{case}: {}", stdout(&output));
        assert!(calls.is_empty(), "{case}: {calls:?}");
        assert!(rig.picture_kept(), "{case}");
    }
}

// ---- M6 -------------------------------------------------------------------------------------

/// A host for the M6 cleanup block: the new stack in `opt/bookclub` (its `.env` names the data
/// directory), the old data directory with `bookclub.db`, and `docker` stood in for, whose
/// healthcheck passes unless `$RIG/health-status` says otherwise.
struct CleanupRig {
    dir: tempfile::TempDir,
}

const CLEANUP_DOCKER: &str = r#"() {
    printf 'docker %s\n' "$*" >>"$RIG/calls.log"
    case "$*" in
        "info -f {{.DockerRootDir}}") cat "$RIG/docker-root" ;;
        "compose exec -T app bookclub-server healthcheck")
            return "$(cat "$RIG/health-status" 2>/dev/null || echo 0)" ;;
        "compose ps -aq bookclub") echo old-container ;;
        "inspect -f {{.Image}} old-container") echo sha256:old ;;
        "image inspect -f "*) printf 'registry.test/bookclub:0.10\n\n' ;;
        "compose rm --stop --force bookclub" | "image rm "*) ;;
        *) echo "unexpected: docker $*" >&2; return 99 ;;
    esac
}"#;

const HEALTHCHECK: &str = "docker compose exec -T app bookclub-server healthcheck";
const REMOVE: &str = "docker compose rm --stop --force bookclub";

impl CleanupRig {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }

    /// Runs the block with `OLD_DATA` (which gets a `bookclub.db`), the stack's data directory
    /// and Docker's root, each relative to the rig.
    fn run(&self, old_data: &str, data: &str, docker_root: &str) -> (Output, Vec<String>) {
        let rooted = |path: &str| self.path(path).display().to_string();
        write(&self.path(old_data).join("bookclub.db"), "old");
        std::fs::create_dir_all(self.path(data).join("media")).unwrap();
        write(
            &self.path("opt/bookclub/.env"),
            &format!("BOOKCLUB_PORT=8000\nBOOKCLUB_DATA_DIR={}\n", rooted(data)),
        );
        write(
            &self.path("docker-root"),
            &format!("{}\n", rooted(docker_root)),
        );
        std::fs::create_dir_all(self.path("old")).unwrap();
        let block = heredoc("CLEANUP")
            .replace("<NEW>", &rooted("opt/bookclub"))
            .replace("<OLD>", &rooted("old"))
            .replace("<OLD_DATA>", &rooted(old_data));
        let _ = std::fs::remove_file(self.path("calls.log"));
        let output = bash_s(
            &block,
            self.dir.path(),
            &[
                ("RIG", self.dir.path().as_os_str()),
                ("BASH_FUNC_docker%%", CLEANUP_DOCKER.as_ref()),
            ],
        );
        (output, calls(self.dir.path()))
    }
}

/// M6 deletes the old data directory whole, after its checks. An old data directory that holds
/// the new stack's directory, its data directory or Docker's root (an old `BOOKCLUB_DATA=/srv`
/// holds `/srv/bookclub`) stops the block before the healthcheck and before anything is removed,
/// and so does a new app that does not answer.
#[test]
fn the_cleanup_keeps_an_old_data_directory_that_holds_the_new_stack() {
    let rig = CleanupRig::new();
    let (output, calls) = rig.run("old/data", "srv/bookclub", "docker");
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!rig.path("old/data").exists());
    assert!(rig.path("srv/bookclub/media").is_dir());
    assert!(rig.path("opt/bookclub/.env").exists());
    let check = calls
        .iter()
        .position(|call| call == HEALTHCHECK)
        .unwrap_or_else(|| panic!("{calls:#?}"));
    let rm = calls
        .iter()
        .position(|call| call == REMOVE)
        .unwrap_or_else(|| panic!("{calls:#?}"));
    assert!(check < rm, "{calls:#?}");
    assert!(calls.contains(&"docker image rm registry.test/bookclub:0.10".to_owned()));

    let rig = CleanupRig::new();
    write(&rig.path("health-status"), "1\n");
    let (output, calls) = rig.run("old/data", "srv/bookclub", "docker");
    assert!(!output.status.success());
    assert!(
        stdout(&output).contains("stop: the new app does not answer"),
        "{}",
        stdout(&output)
    );
    assert!(rig.path("old/data/bookclub.db").exists());
    assert!(
        !calls.iter().any(|call| call.contains(" rm ")),
        "{calls:#?}"
    );

    for (case, old_data, data, docker_root, held) in [
        (
            "the data directory",
            "srv",
            "srv/bookclub",
            "docker",
            "srv/bookclub",
        ),
        (
            "the new stack",
            "opt",
            "srv/bookclub",
            "docker",
            "opt/bookclub",
        ),
        (
            "Docker's root",
            "old/data",
            "srv/bookclub",
            "old/data/docker",
            "old/data/docker",
        ),
        ("everything", ".", "srv/bookclub", "docker", "opt/bookclub"),
    ] {
        let rig = CleanupRig::new();
        let (output, calls) = rig.run(old_data, data, docker_root);
        assert!(!output.status.success(), "{case}");
        let printed = stdout(&output);
        let held = rig.path(held).display().to_string();
        assert!(
            printed.contains("stop: ") && printed.contains(&format!("holds {held};")),
            "{case}: {printed}"
        );
        assert!(rig.path(old_data).join("bookclub.db").exists(), "{case}");
        assert!(
            !calls
                .iter()
                .any(|call| call == HEALTHCHECK || call.contains(" rm ")),
            "{case}: {calls:#?}"
        );
    }
}

// ---- host-facts.sh --------------------------------------------------------------------------

/// `docker` for `host-facts.sh`: a recent engine and compose, one old container whose `/data`
/// mount is `$FAKE_DATA`, and nothing else.
const FACTS_DOCKER: &str = r#"() {
    case "$*" in
        "version --format "*) echo 27.3.1 ;;
        "compose version --short") echo 2.29.7 ;;
        "info --format "*) echo /var/lib/docker ;;
        "ps -aq "*) echo c0ffee ;;
        *'"/data"'*) printf '%s\n' "$FAKE_DATA" ;;
        *State.Pid*) echo 0 ;;
    esac
}"#;

const FACTS_CURL: &str = r#"() { printf 'HTTP/2 200\r\n'; }"#;

fn host_facts_notes(data: &str) -> Vec<String> {
    let old = tempfile::tempdir().unwrap();
    let output = Command::new("bash")
        .arg(repo().join("deploy/cutover/host-facts.sh"))
        .arg(old.path())
        .arg("books.example.test")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("PEER_SECONDS", "0")
        .env("FAKE_DATA", data)
        .env("BASH_FUNC_docker%%", FACTS_DOCKER)
        .env("BASH_FUNC_curl%%", FACTS_CURL)
        .output()
        .unwrap();
    let printed = stdout(&output);
    assert!(
        printed.contains("== Checks =="),
        "{printed}{}",
        stderr(&output)
    );
    printed
        .lines()
        .filter_map(|line| line.strip_prefix("note: the old data directory "))
        .map(str::to_owned)
        .collect()
}

/// An old data directory that holds a path of the new layout gets a note, so 2.3 picks another
/// one before anything is installed; a directory of its own gets none.
#[test]
fn host_facts_notes_an_old_data_directory_that_holds_the_new_layout() {
    let notes = host_facts_notes("/srv");
    let held: Vec<&str> = notes
        .iter()
        .map(|note| {
            note.strip_prefix("/srv holds ")
                .and_then(|rest| rest.split(':').next())
                .unwrap_or_else(|| panic!("{note}"))
        })
        .collect();
    assert_eq!(
        held,
        [
            "/srv/bookclub",
            "/srv/bookclub-server",
            "/srv/bookclub-backup"
        ]
    );
    assert_eq!(host_facts_notes("/").len(), 5);
    assert!(host_facts_notes("/opt/python-bookclub/data").is_empty());
}
