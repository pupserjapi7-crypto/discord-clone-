//! PostgreSQL access layer.
//!
//! All queries in the request path use `sqlx` compile-time checked macros.

pub mod models;
pub mod repo;
pub mod views;

pub use models::*;
pub use repo::PermCache;

use std::time::Duration;

use anyhow::Context;
use sqlx::postgres::{PgPool, PgPoolOptions};

#[derive(Clone)]
pub struct Db {
    pub pool: PgPool,
}

impl Db {
    pub async fn connect(database_url: &str, max_connections: u32) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(Duration::from_secs(5))
            .connect(database_url)
            .await
            .context("connect to PostgreSQL")?;
        Ok(Self { pool })
    }

    /// Run embedded migrations. Safe to call on every start.
    pub async fn migrate(&self) -> anyhow::Result<()> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .context("run migrations")?;
        Ok(())
    }

    pub async fn ping(&self) -> Result<(), sqlx::Error> {
        sqlx::query("select 1")
            .execute(&self.pool)
            .await
            .map(|_| ())
    }
}
