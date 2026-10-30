//! The private move's runbook (MOVE #52, `docs/phase3/private-move.md`) where its blocks are code
//! that runs as root on the production host. Each test reads the block from the runbook as it
//! stands:
//! - `summary` (section 0) prints a report's lines before the first section only when it knows
//!   them, and each section as its title and a count: no line of a section reaches the terminal,
//!   also one that a title with a line break starts without indentation.
//! - The rollback's first step (section 8) counts the accounts other than the club's owner that
//!   signed in to the new app, by a live session or by one that ended.
//! - The rollback's delete block stops before it touches anything when the move's variables are
//!   missing or `$NEW` is not the new stack, and deletes nothing when the backup fails.
//!
//! `bash`, `awk` and `find` are the real ones. `docker` is a bash function imported from the
//! environment that logs its calls, and `backup.sh` is a stub script.

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

fn executable(path: &Path, content: &str) {
    use std::os::unix::fs::PermissionsExt;

    write(path, content);
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
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

async fn window(db: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
        .fetch_one(db)
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
}

/// The rollback's first step: 0 while only the club's owner (Rafael) signed in, and above 0
/// once another account signed in, also after its session ended (a logout leaves
/// `last_seen_at`).
#[sqlx::test]
async fn the_rollback_window_counts_sign_ins_besides_the_owner(db: PgPool) {
    let sql = heredoc("SQL");
    assert_eq!(window(&db, &sql).await, 0, "an empty target");

    let mut tx = db.begin().await.unwrap();
    let owner: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash) VALUES ('owner1', 'x') RETURNING id",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let member: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash) VALUES ('member1', 'x') RETURNING id",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
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
    assert_eq!(window(&db, &sql).await, 0, "imported, nobody signed in");

    let session = "INSERT INTO sessions (user_id, kind, token_hash, expires_at)
                   VALUES ($1, 'cookie', decode(repeat($2, 32), 'hex'), now() + interval '30 days')";
    sqlx::query(session)
        .bind(owner)
        .bind("ab")
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(window(&db, &sql).await, 0, "Rafael's checks");

    sqlx::query(session)
        .bind(member)
        .bind("cd")
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(window(&db, &sql).await, 1, "the other member signed in");

    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(member)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(window(&db, &sql).await, 1, "and signed out again");
}

/// A host for the rollback's delete block: `new/` with `compose.yaml`, `backup.env` and a stub
/// `backup.sh`, `data/media` with a picture, `data/import` with the copy, and `old/` with only
/// a `compose.yaml` (the Python app's directory).
struct RollbackRig {
    dir: tempfile::TempDir,
}

const LOGGING_DOCKER: &str = r#"() { printf 'docker %s\n' "$*" >>"$RIG/calls.log"; }"#;

impl RollbackRig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("new/compose.yaml"), "name: bookclub\n");
        write(
            &root.join("new/backup.env"),
            &format!("MEDIA_DIR={}/data/media\n", root.display()),
        );
        executable(
            &root.join("new/backup.sh"),
            "#!/usr/bin/env bash\nprintf 'backup.sh\\n' >>\"$RIG/calls.log\"\n\
             exit \"$(cat \"$RIG/backup-status\" 2>/dev/null || echo 0)\"\n",
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

/// The block backs up, then removes the new stack with its volume and empties its media
/// directory; the copy in `$IMPORT` waits for step 6. It deletes nothing when the backup fails,
/// when `vars.sh` cannot be read or misses `DATA`, when `$NEW` is not the new stack, or when
/// the stack's `MEDIA_DIR` is not `$DATA/media`.
#[test]
fn the_rollback_deletes_only_the_new_stack_and_only_after_its_backup() {
    let rig = RollbackRig::new();
    let (output, calls) = rig.run();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(calls, ["backup.sh", "docker compose down -v"]);
    assert!(!rig.picture_kept());
    assert!(
        rig.root().join("data/media").is_dir(),
        "the directory stays"
    );
    assert!(rig.root().join("data/import/bookclub.db").exists());

    let rig = RollbackRig::new();
    write(&rig.root().join("backup-status"), "1\n");
    let (output, calls) = rig.run();
    assert!(!output.status.success());
    assert_eq!(calls, ["backup.sh"], "a failed backup stops the block");
    assert!(rig.picture_kept());

    let broken: [(&str, Breakage); 4] = [
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
        ("MEDIA_DIR is not $DATA/media", |rig| {
            write(
                &rig.root().join("new/backup.env"),
                &format!("MEDIA_DIR={}/elsewhere/media\n", rig.root().display()),
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
