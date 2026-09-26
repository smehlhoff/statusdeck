use anyhow::Result;
use statusdeck_backend::{
    check_worker_health, config::Config, logging, run_api, run_migrations, run_worker,
};

#[tokio::main]
async fn main() -> Result<()> {
    let command = std::env::args().nth(1).unwrap_or_else(|| "api".to_owned());
    if command == "validate-catalog" {
        return statusdeck_backend::providers::catalog::validate();
    }
    if command == "migrate" {
        return run_migrations(&Config::database_url_from_env()?).await;
    }
    if command == "healthcheck-worker" {
        return check_worker_health(&Config::database_url_from_env()?).await;
    }
    let config = Config::from_env()?;
    logging::init(&config);

    match command.as_str() {
        "api" => run_api(config).await,
        "worker" => run_worker(config).await,
        other => Err(anyhow::anyhow!(
            "unknown command: {other}; expected api, worker, migrate, healthcheck-worker, or validate-catalog"
        )),
    }
}
