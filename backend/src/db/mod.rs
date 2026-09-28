use anyhow::{Context, Result};
use sqlx::{PgPool, postgres::PgPoolOptions};

use crate::config::Config;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[must_use]
pub fn expected_migration_version() -> i64 {
    MIGRATOR
        .iter()
        .last()
        .map_or(0, |migration| migration.version)
}

#[derive(Clone)]
pub struct Database {
    pub pool: PgPool,
}

impl Database {
    pub async fn connect(config: &Config) -> Result<Self> {
        let maximum_connections = u32::try_from(config.global_poll_concurrency.min(23) + 7)
            .context("configured concurrency is too large")?;
        let pool = PgPoolOptions::new()
            .max_connections(maximum_connections)
            .acquire_timeout(std::time::Duration::from_secs(10))
            .connect(&config.database_url)
            .await
            .context("could not connect to PostgreSQL")?;
        Ok(Self { pool })
    }

    pub async fn migrations_current(&self) -> bool {
        sqlx::query_as::<_, (i64, bool, Vec<u8>)>(
            "SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version",
        )
        .fetch_all(&self.pool)
        .await
        .is_ok_and(|applied| {
            applied.len() == MIGRATOR.iter().count()
                && applied.iter().zip(MIGRATOR.iter()).all(
                    |((version, successful, checksum), expected)| {
                        *successful
                            && *version == expected.version
                            && checksum.as_slice() == expected.checksum.as_ref()
                    },
                )
        })
    }

    pub async fn verify_migrations(&self) -> Result<()> {
        if !self.migrations_current().await {
            anyhow::bail!(
                "database migrations are missing or incompatible; run the migrate command"
            );
        }
        Ok(())
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}
