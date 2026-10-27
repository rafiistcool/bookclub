use anyhow::{Context, Result};
use bookclub_server::config::Config;
use bookclub_server::{AppState, app};
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env()?;
    // Connect lazily so the server starts (and reports 503 on /api/health)
    // even while Postgres is still coming up.
    let db = PgPoolOptions::new()
        .connect_lazy(&config.database_url)
        .context("DATABASE_URL is not a valid Postgres connection string")?;

    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .with_context(|| format!("cannot listen on {}", config.bind_addr))?;
    tracing::info!("listening on http://{}", config.bind_addr);

    axum::serve(listener, app(AppState { db }))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
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
