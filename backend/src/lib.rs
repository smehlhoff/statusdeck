#![forbid(unsafe_code)]

pub mod api;
pub mod auth;
pub mod config;
pub mod db;
pub mod domain;
pub mod logging;
pub mod notifications;
pub mod polling;
pub mod providers;
mod retry_after;

use anyhow::Result;
use config::Config;
use db::Database;

pub async fn run_migrations(database_url: &str) -> Result<()> {
    let pool = sqlx::PgPool::connect(database_url).await?;
    db::MIGRATOR.run(&pool).await?;
    pool.close().await;
    Ok(())
}

pub async fn check_worker_health(database_url: &str) -> Result<()> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(database_url)
        .await?;
    let healthy = sqlx::query_scalar::<_, bool>("SELECT count(DISTINCT role) = 2 FROM worker_heartbeats WHERE role IN ('poller', 'dispatcher') AND heartbeat_at > now() - interval '2 minutes'")
        .fetch_one(&pool)
        .await?;
    pool.close().await;
    anyhow::ensure!(healthy, "worker heartbeat is stale");
    Ok(())
}

pub async fn run_api(config: Config) -> Result<()> {
    let database = Database::connect(&config).await?;
    database.verify_migrations().await?;
    auth::bootstrap::ensure_admin(&database.pool, &config).await?;
    providers::catalog::reconcile(&database.pool, config.poll_interval).await?;
    let app = api::router(config.clone(), database.clone()).await?;
    let listener = tokio::net::TcpListener::bind(&config.bind).await?;
    tracing::info!(address = %config.bind, "api listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    database.close().await;
    Ok(())
}

pub async fn run_worker(config: Config) -> Result<()> {
    let database = Database::connect(&config).await?;
    database.verify_migrations().await?;
    auth::bootstrap::ensure_admin(&database.pool, &config).await?;
    providers::catalog::reconcile(&database.pool, config.poll_interval).await?;
    polling::run(config, database).await
}

pub(crate) async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "could not install Ctrl-C handler");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::error!(%error, "could not install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
