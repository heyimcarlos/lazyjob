use sqlx::postgres::PgPoolOptions;
use sqlx::{Connection, Executor, PgConnection, PgPool};
use uuid::Uuid;

use crate::db::DEFAULT_DATABASE_URL;

pub struct TestDb {
    pool: PgPool,
    db_name: String,
    server_url: String,
}

impl TestDb {
    pub async fn spawn() -> Self {
        let server_url =
            std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_string());

        let server_url = server_url
            .rsplit_once('/')
            .map(|(base, _)| base.to_string())
            .unwrap_or(server_url);

        let db_name = format!("test_{}", Uuid::new_v4().to_string().replace('-', ""));

        let mut conn = PgConnection::connect(&format!("{}/postgres", server_url))
            .await
            .expect("Failed to connect to Postgres server");

        conn.execute(format!(r#"CREATE DATABASE "{}""#, db_name).as_str())
            .await
            .expect("Failed to create test database");

        let database_url = format!("{}/{}", server_url, db_name);
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("Failed to connect to test database");

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("Failed to run migrations on test database");

        Self {
            pool,
            db_name,
            server_url,
        }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let server_url = self.server_url.clone();
        let db_name = self.db_name.clone();
        let pool = self.pool.clone();

        // Fire-and-forget: spawn a detached thread to clean up.
        // We do NOT join — joining blocks the tokio runtime thread and can deadlock.
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                pool.close().await;

                if let Ok(mut conn) =
                    PgConnection::connect(&format!("{}/postgres", server_url)).await
                {
                    let _ = conn
                        .execute(
                            format!(
                                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '{}'",
                                db_name
                            )
                            .as_str(),
                        )
                        .await;

                    let _ = conn
                        .execute(format!(r#"DROP DATABASE IF EXISTS "{}""#, db_name).as_str())
                        .await;
                }
            });
        });
    }
}
