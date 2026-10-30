//! The private move's runbook (MOVE #52, `docs/phase3/private-move.md`) where its blocks are code
//! that runs as root on the production host. Each test reads the block from the runbook as it
//! stands:
//! - `summary` (section 0) prints a report's lines before the first section only when it knows
//!   them, and each section as its title and a count: no line of a section reaches the terminal,
//!   also one that a title with a line break starts without indentation.
//!
//! `bash` and `awk` are the real ones.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use bookclub_server::importer::Report;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}

fn runbook() -> String {
    std::fs::read_to_string(repo().join("docs/phase3/private-move.md")).unwrap()
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
