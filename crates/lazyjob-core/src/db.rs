use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use crate::error::Result;

pub const DEFAULT_DATABASE_URL: &str = "postgresql://localhost/lazyjob";

pub struct Database {
    pool: PgPool,
}

impl Database {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;

        sqlx::migrate!("./migrations").run(&pool).await?;

        Ok(Self { pool })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn close(self) {
        self.pool.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // learning test: verifies sqlx PgPoolOptions can be configured without a connection
    #[test]
    fn pg_pool_options_configurable() {
        let opts = PgPoolOptions::new().max_connections(10).min_connections(1);
        // PgPoolOptions is a builder — this proves we can configure it without panicking
        drop(opts);
    }

    // learning test: verifies sqlx::migrate!() macro compiles and embeds migration files
    #[test]
    fn migration_files_embedded() {
        let migrator = sqlx::migrate!("./migrations");
        assert!(
            !migrator.migrations.is_empty(),
            "expected at least one migration file to be embedded"
        );
        let first = &migrator.migrations[0];
        assert_eq!(first.version, 1);
    }

    #[test]
    fn default_database_url_is_valid() {
        assert!(DEFAULT_DATABASE_URL.starts_with("postgresql://"));
    }

    #[tokio::test]
    async fn connect_and_migrate() {
        let database_url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!("Skipping connect_and_migrate: DATABASE_URL not set");
                return;
            }
        };

        let db = Database::connect(&database_url).await.unwrap();

        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT tablename::text FROM pg_tables WHERE schemaname = 'public' ORDER BY tablename",
        )
        .fetch_all(db.pool())
        .await
        .unwrap();

        let table_names: Vec<&str> = tables.iter().map(|t| t.0.as_str()).collect();

        let expected = [
            "application_transitions",
            "applications",
            "companies",
            "contacts",
            "interviews",
            "jobs",
            "life_sheet_items",
            "offers",
            "ralph_loop_runs",
            "token_usage_log",
        ];

        for expected_table in &expected {
            assert!(
                table_names.contains(expected_table),
                "missing table: {expected_table}"
            );
        }

        db.close().await;
    }
}
