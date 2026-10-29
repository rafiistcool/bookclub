use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use bookclub_server::auth::rules::valid_email;
use bookclub_server::clubs::out::invite_out;
use bookclub_server::config::{Config, EmailTransport};
use bookclub_server::email::{self, EmailSender, LogEmail, SmtpEmail};
use bookclub_server::importer::{self, ImportError};
use bookclub_server::media::shared::FsMediaStore;
use bookclub_server::{AppState, admin, app, jobs, serve};
use clap::{Parser, Subcommand};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Connection, PgConnection, PgPool};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

mod healthcheck;

const CONNECT_RETRY_EVERY: Duration = Duration::from_secs(2);
const CONNECT_GIVE_UP_AFTER: Duration = Duration::from_secs(60);
/// How long shutdown waits for background jobs to stop.
const JOBS_STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// The Bookclub server and its maintenance commands. Configuration comes from the environment
/// (docs/phase1/conventions.md §11).
#[derive(Parser)]
#[command(name = "bookclub-server", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the HTTP server (the default).
    Serve,
    /// Import the legacy SQLite database into an empty Postgres database.
    ImportLegacy(importer::Args),
    /// Re-resolve legacy bare-facts editions by ISBN.
    RematchLegacy(importer::RematchArgs),
    /// Make an existing user an instance admin.
    MakeAdmin {
        /// The user's name.
        username: String,
    },
    /// End an instance admin's rights (a suspension ends them too).
    RevokeAdmin {
        /// The user's name.
        username: String,
    },
    /// Lift an account's suspension, as "Lift suspension" on the admin page does: it signs in
    /// again, without the admin rights the suspension ended.
    Unsuspend {
        /// The user's name.
        username: String,
    },
    /// Mint a closed-beta sign-up invite and print its link.
    BetaInvite {
        /// How many sign-ups the invite allows (1 to 100).
        #[arg(long, value_parser = clap::value_parser!(i32).range(1..=100))]
        uses: Option<i32>,
        /// Days until the invite expires (1 to 90).
        #[arg(long, value_parser = clap::value_parser!(i32).range(1..=90))]
        days: Option<i32>,
    },
    /// Print the OpenAPI document as JSON. Needs no database.
    Openapi,
    /// Ask the running server's /api/health on the address BIND_ADDR names (the image's
    /// HEALTHCHECK). Exits 0 only on 200. Reads no other setting and needs no database.
    Healthcheck,
    /// Send one test mail through the configured transport (SMTP_URL, EMAIL_FROM). Exits
    /// non-zero when sending fails. Needs no database.
    SendTestMail {
        /// The recipient address.
        address: String,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        // The log goes to stderr, so a command's stdout holds only its result (beta-invite's
        // code and link, openapi's JSON, the importer's report). Colours only on a terminal:
        // Docker's log files keep plain text.
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();

    let cli = Cli::parse();
    let result = match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => serve().await,
        Command::ImportLegacy(args) => return import_legacy(args).await,
        Command::RematchLegacy(args) => return rematch_legacy(args).await,
        Command::MakeAdmin { username } => make_admin(&username).await,
        Command::RevokeAdmin { username } => revoke_admin(&username).await,
        Command::Unsuspend { username } => unsuspend(&username).await,
        Command::BetaInvite { uses, days } => beta_invite(uses, days).await,
        Command::Openapi => print_openapi(),
        Command::Healthcheck => healthcheck::run().await,
        Command::SendTestMail { address } => send_test_mail(&address).await,
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn serve() -> Result<()> {
    let config = Config::from_env()?;
    check_media_dir(&config).await?;
    let db = connect(&config).await?;
    let bind_addr = config.bind_addr;
    let state = AppState::new(db, config)?;

    let shutdown = CancellationToken::new();
    let mut background = jobs::spawn_all(state.clone(), shutdown.clone());

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("cannot listen on {bind_addr}"))?;
    tracing::info!("listening on http://{bind_addr}");

    let stop = shutdown.clone();
    serve::run(
        listener,
        app(state),
        serve::HEADER_READ_TIMEOUT,
        async move {
            shutdown_signal().await;
            stop.cancel();
        },
    )
    .await;

    shutdown.cancel();
    if tokio::time::timeout(JOBS_STOP_TIMEOUT, async {
        while background.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        tracing::warn!("background jobs did not stop in time; aborting them");
        background.abort_all();
    }
    Ok(())
}

/// Fails with `MEDIA_DIR <path> is not writable` unless the media directory can be created and
/// written. Runs before the database connection, so a wrong mount fails at once.
async fn check_media_dir(config: &Config) -> Result<()> {
    FsMediaStore::new(config.media_dir.clone())
        .check_writable()
        .await
}

/// Connects to Postgres, retrying every 2 s for up to 60 s (the database may still be starting),
/// then applies the migrations.
async fn connect(config: &Config) -> Result<PgPool> {
    let options: PgConnectOptions = config
        .database_url
        .parse()
        .context("DATABASE_URL is not a valid Postgres connection string")?;
    // Probe with a single connection: a pool would keep retrying a refused connection for its
    // whole acquire timeout before reporting anything.
    let started = tokio::time::Instant::now();
    loop {
        match PgConnection::connect_with(&options).await {
            Ok(probe) => {
                let _ = probe.close().await;
                break;
            }
            Err(error) if started.elapsed() < CONNECT_GIVE_UP_AFTER => {
                tracing::warn!(%error, "cannot reach the database yet; retrying");
                tokio::time::sleep(CONNECT_RETRY_EVERY).await;
            }
            Err(error) => {
                return Err(error).context("cannot reach the database (gave up after 60 s)");
            }
        }
    }
    let db = PgPoolOptions::new()
        .connect_with(options)
        .await
        .context("cannot open the database connection pool")?;
    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .context("cannot apply the database migrations")?;
    Ok(db)
}

async fn import_legacy(args: importer::Args) -> ExitCode {
    let state = match connected_state(Media::Writes).await {
        Ok(state) => state,
        Err(error) => return fail(&error),
    };
    let result = importer::run(
        state.db.clone(),
        state.lookup.clone(),
        state.media.clone(),
        args,
    )
    .await;
    report(result)
}

async fn rematch_legacy(args: importer::RematchArgs) -> ExitCode {
    let state = match connected_state(Media::Untouched).await {
        Ok(state) => state,
        Err(error) => return fail(&error),
    };
    report(importer::rematch(state.db.clone(), state.lookup.clone(), args).await)
}

/// The production state for the maintenance commands: the same configuration as `serve`. A
/// command that stores media passes `Media::Writes`, which checks `MEDIA_DIR` first, as `serve`
/// does.
async fn connected_state(media: Media) -> Result<AppState> {
    let config = Config::from_env()?;
    if media == Media::Writes {
        check_media_dir(&config).await?;
    }
    let db = connect(&config).await?;
    AppState::new(db, config)
}

/// Whether a maintenance command stores media.
#[derive(PartialEq, Eq)]
enum Media {
    Writes,
    Untouched,
}

/// Prints an importer report; `TargetNotEmpty` exits with 2, other errors with 1.
fn report(result: Result<importer::Report, ImportError>) -> ExitCode {
    match result {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(ImportError::TargetNotEmpty) => {
            eprintln!("error: {}", ImportError::TargetNotEmpty);
            ExitCode::from(2)
        }
        Err(error) => fail(&anyhow::Error::new(error)),
    }
}

fn fail(error: &anyhow::Error) -> ExitCode {
    eprintln!("error: {error:#}");
    ExitCode::FAILURE
}

async fn make_admin(username: &str) -> Result<()> {
    let config = Config::from_env()?;
    let db = connect(&config).await?;
    let mut conn = db.acquire().await?;
    if admin::make_admin(&mut conn, username).await? {
        println!(
            "{} is now an instance admin",
            username.trim().to_lowercase()
        );
        Ok(())
    } else {
        anyhow::bail!("no user named {username:?}")
    }
}

async fn revoke_admin(username: &str) -> Result<()> {
    let config = Config::from_env()?;
    let db = connect(&config).await?;
    let mut conn = db.acquire().await?;
    if admin::revoke_admin(&mut conn, username).await? {
        println!(
            "{} is no longer an instance admin",
            username.trim().to_lowercase()
        );
        Ok(())
    } else {
        anyhow::bail!("no user named {username:?}")
    }
}

/// Lifts a suspension from the host, where the admin page cannot: when the only admin is
/// suspended, for one.
async fn unsuspend(username: &str) -> Result<()> {
    let config = Config::from_env()?;
    let db = connect(&config).await?;
    let mut conn = db.acquire().await?;
    match admin::users::lift_suspension(&mut conn, username).await? {
        Some(id) => {
            tracing::info!(user = id, "account unsuspended from the command line");
            println!("{} can sign in again", username.trim().to_lowercase());
            Ok(())
        }
        None => anyhow::bail!("no user named {username:?}"),
    }
}

/// Mints a closed-beta sign-up invite (no club) and prints its code and join link. Without
/// flags it is single-use and expires in 30 days.
async fn beta_invite(uses: Option<i32>, days: Option<i32>) -> Result<()> {
    let config = Config::from_env()?;
    let db = connect(&config).await?;
    let mut conn = db.acquire().await?;
    let id = admin::invites::mint_signup_invite(&mut conn, None, uses, days).await?;
    let invite = invite_out(&mut conn, &config, None, id)
        .await
        .map_err(|error| anyhow::anyhow!("cannot read the new invite: {}", error.detail))?;
    println!("{}", invite.code);
    println!("{}", invite.url);
    Ok(())
}

/// Sends [`email::test_mail`] to `address` through the configured transport. It reads the full
/// configuration, as `serve` does, but never connects to the database.
async fn send_test_mail(address: &str) -> Result<()> {
    let config = Config::from_env()?;
    let address = address.trim();
    if !valid_email(address) {
        bail!("{address:?} is not a plain email address, e.g. ada@example.com");
    }
    let sender: Box<dyn EmailSender> = match config.email_transport {
        EmailTransport::Log => Box::new(LogEmail),
        EmailTransport::Smtp => Box::new(SmtpEmail::new(&config)?),
    };
    sender.send(email::test_mail(&config, address)).await?;
    println!("sent a test mail to {address}");
    Ok(())
}

fn print_openapi() -> Result<()> {
    let json = bookclub_server::openapi()
        .to_pretty_json()
        .context("cannot serialise the OpenAPI document")?;
    println!("{json}");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("cannot listen for Ctrl+C");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("cannot listen for SIGTERM")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutting down");
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    /// clap checks a subcommand's arguments only when that subcommand is parsed; this checks
    /// all of them, including the flags IMP adds to the flattened importer arguments.
    #[test]
    fn the_cli_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn import_legacy_reads_its_flags() {
        let cli = Cli::try_parse_from([
            "bookclub-server",
            "import-legacy",
            "old/bookclub.db",
            "--owner",
            "ada",
            "--club-name",
            "Thursday Readers",
            "--timezone",
            "Europe/Berlin",
            "--dry-run",
            "--allow-unmatched",
        ])
        .unwrap();
        let Some(Command::ImportLegacy(args)) = cli.command else {
            panic!("not import-legacy");
        };
        assert_eq!(args.source, std::path::PathBuf::from("old/bookclub.db"));
        assert_eq!(args.owner, "ada");
        assert_eq!(args.club_name, "Thursday Readers");
        assert_eq!(args.timezone, "Europe/Berlin");
        assert_eq!(args.club_slug, None);
        assert!(args.dry_run && args.allow_unmatched && !args.offline);

        let cli = Cli::try_parse_from(["bookclub-server", "rematch-legacy", "--dry-run"]).unwrap();
        assert!(matches!(cli.command, Some(Command::RematchLegacy(args)) if args.dry_run));
        let cli = Cli::try_parse_from(["bookclub-server"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn the_deploy_commands_parse() {
        let cli = Cli::try_parse_from(["bookclub-server", "healthcheck"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Healthcheck)));
        let cli =
            Cli::try_parse_from(["bookclub-server", "send-test-mail", "ada@example.com"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::SendTestMail { address }) if address == "ada@example.com"
        ));
        assert!(Cli::try_parse_from(["bookclub-server", "send-test-mail"]).is_err());
    }

    #[test]
    fn the_account_commands_parse() {
        let cli = Cli::try_parse_from(["bookclub-server", "unsuspend", "ada"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Unsuspend { username }) if username == "ada"));
        let cli = Cli::try_parse_from(["bookclub-server", "revoke-admin", "eve"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::RevokeAdmin { username }) if username == "eve"
        ));
        assert!(Cli::try_parse_from(["bookclub-server", "unsuspend"]).is_err());
    }
}
