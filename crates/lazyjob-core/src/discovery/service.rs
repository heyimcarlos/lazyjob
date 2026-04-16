use futures::future::join_all;
use sqlx::PgPool;
use tokio::sync::mpsc;

use crate::discovery::sources::{GreenhouseClient, JobSource, LeverClient};
use crate::error::Result;
use crate::repositories::JobRepository;

pub struct SourceConfig {
    pub source: String,
    pub company_id: String,
}

#[derive(Debug, Default, Clone)]
pub struct DiscoveryStats {
    pub jobs_found: usize,
    pub jobs_new: usize,
    pub jobs_updated: usize,
    pub errors: usize,
}

impl std::ops::Add for DiscoveryStats {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self {
            jobs_found: self.jobs_found + rhs.jobs_found,
            jobs_new: self.jobs_new + rhs.jobs_new,
            jobs_updated: self.jobs_updated + rhs.jobs_updated,
            errors: self.errors + rhs.errors,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiscoveryProgress {
    pub source: String,
    pub company_id: String,
    pub message: String,
}

pub struct DiscoveryService;

impl DiscoveryService {
    pub async fn run_discovery(
        pool: &PgPool,
        sources: Vec<SourceConfig>,
        progress_tx: Option<mpsc::Sender<DiscoveryProgress>>,
    ) -> Result<DiscoveryStats> {
        let futs: Vec<_> = sources
            .into_iter()
            .map(|cfg| {
                let pool = pool.clone();
                let tx = progress_tx.clone();
                async move { discover_one(pool, cfg, tx).await }
            })
            .collect();

        let results = join_all(futs).await;

        let stats = results
            .into_iter()
            .fold(DiscoveryStats::default(), |acc, r| acc + r);

        Ok(stats)
    }
}

async fn discover_one(
    pool: PgPool,
    cfg: SourceConfig,
    progress_tx: Option<mpsc::Sender<DiscoveryProgress>>,
) -> DiscoveryStats {
    let client: Box<dyn JobSource> = match cfg.source.as_str() {
        "greenhouse" => Box::new(GreenhouseClient::new()),
        "lever" => Box::new(LeverClient::new()),
        other => {
            send_progress(
                &progress_tx,
                &cfg.source,
                &cfg.company_id,
                &format!("unknown source '{other}', skipping"),
            )
            .await;
            return DiscoveryStats {
                errors: 1,
                ..Default::default()
            };
        }
    };

    send_progress(
        &progress_tx,
        &cfg.source,
        &cfg.company_id,
        &format!("fetching jobs from {} / {}", cfg.source, cfg.company_id),
    )
    .await;

    let jobs = match client.fetch_jobs(&cfg.company_id).await {
        Ok(jobs) => jobs,
        Err(e) => {
            send_progress(
                &progress_tx,
                &cfg.source,
                &cfg.company_id,
                &format!("fetch error: {e}"),
            )
            .await;
            return DiscoveryStats {
                errors: 1,
                ..Default::default()
            };
        }
    };

    let jobs_found = jobs.len();
    let repo = JobRepository::new(pool);
    let mut jobs_new = 0usize;
    let mut jobs_updated = 0usize;
    let mut errors = 0usize;

    for job in &jobs {
        match repo.upsert_discovered(job).await {
            Ok(true) => jobs_new += 1,
            Ok(false) => jobs_updated += 1,
            Err(e) => {
                tracing::warn!("upsert failed for job {}: {e}", job.id);
                errors += 1;
            }
        }
    }

    send_progress(
        &progress_tx,
        &cfg.source,
        &cfg.company_id,
        &format!(
            "done: {jobs_found} found, {jobs_new} new, {jobs_updated} updated, {errors} errors"
        ),
    )
    .await;

    DiscoveryStats {
        jobs_found,
        jobs_new,
        jobs_updated,
        errors,
    }
}

async fn send_progress(
    tx: &Option<mpsc::Sender<DiscoveryProgress>>,
    source: &str,
    company_id: &str,
    message: &str,
) {
    if let Some(tx) = tx {
        let _ = tx
            .send(DiscoveryProgress {
                source: source.to_string(),
                company_id: company_id.to_string(),
                message: message.to_string(),
            })
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::future::join_all;

    // learning test: verifies library behavior
    #[tokio::test]
    async fn futures_join_all_collects_from_parallel_futures() {
        async fn const_fut(n: usize) -> usize {
            n
        }
        let futs = vec![const_fut(1), const_fut(2), const_fut(3)];
        let results = join_all(futs).await;
        assert_eq!(results, vec![1, 2, 3]);
    }

    #[test]
    fn discovery_stats_add_aggregates_correctly() {
        let a = DiscoveryStats {
            jobs_found: 10,
            jobs_new: 5,
            jobs_updated: 3,
            errors: 2,
        };
        let b = DiscoveryStats {
            jobs_found: 7,
            jobs_new: 7,
            jobs_updated: 0,
            errors: 0,
        };
        let total = a + b;
        assert_eq!(total.jobs_found, 17);
        assert_eq!(total.jobs_new, 12);
        assert_eq!(total.jobs_updated, 3);
        assert_eq!(total.errors, 2);
    }

    #[test]
    fn discovery_stats_default_is_zero() {
        let s = DiscoveryStats::default();
        assert_eq!(s.jobs_found, 0);
        assert_eq!(s.jobs_new, 0);
        assert_eq!(s.jobs_updated, 0);
        assert_eq!(s.errors, 0);
    }

    #[test]
    fn source_config_fields_accessible() {
        let cfg = SourceConfig {
            source: "greenhouse".to_string(),
            company_id: "stripe".to_string(),
        };
        assert_eq!(cfg.source, "greenhouse");
        assert_eq!(cfg.company_id, "stripe");
    }

    #[test]
    fn discovery_progress_fields_accessible() {
        let p = DiscoveryProgress {
            source: "lever".to_string(),
            company_id: "notion".to_string(),
            message: "fetching".to_string(),
        };
        assert_eq!(p.source, "lever");
        assert_eq!(p.company_id, "notion");
    }

    #[tokio::test]
    async fn run_discovery_empty_sources_returns_zero_stats() {
        let pool = match std::env::var("DATABASE_URL") {
            Ok(url) => sqlx::PgPool::connect(&url).await.unwrap(),
            Err(_) => return,
        };
        let stats = DiscoveryService::run_discovery(&pool, vec![], None)
            .await
            .unwrap();
        assert_eq!(stats.jobs_found, 0);
        assert_eq!(stats.errors, 0);
    }

    #[tokio::test]
    async fn discover_one_unknown_source_increments_error() {
        let pool = match std::env::var("DATABASE_URL") {
            Ok(url) => sqlx::PgPool::connect(&url).await.unwrap(),
            Err(_) => return,
        };
        let cfg = SourceConfig {
            source: "unknown_source".to_string(),
            company_id: "acme".to_string(),
        };
        let stats = discover_one(pool, cfg, None).await;
        assert_eq!(stats.errors, 1);
        assert_eq!(stats.jobs_found, 0);
    }

    #[tokio::test]
    async fn run_discovery_sends_progress_events() {
        let pool = match std::env::var("DATABASE_URL") {
            Ok(url) => sqlx::PgPool::connect(&url).await.unwrap(),
            Err(_) => return,
        };
        let (tx, mut rx) = mpsc::channel(32);
        let sources = vec![SourceConfig {
            source: "unknown_source_xyz".to_string(),
            company_id: "test-co".to_string(),
        }];
        DiscoveryService::run_discovery(&pool, sources, Some(tx))
            .await
            .unwrap();

        let msg = rx.recv().await.expect("expected progress event");
        assert_eq!(msg.source, "unknown_source_xyz");
        assert_eq!(msg.company_id, "test-co");
        assert!(msg.message.contains("unknown source"));
    }

    #[tokio::test]
    async fn upsert_discovered_returns_true_for_new() {
        let url = match std::env::var("DATABASE_URL") {
            Ok(u) => u,
            Err(_) => return,
        };
        let pool = sqlx::PgPool::connect(&url).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        let repo = crate::repositories::JobRepository::new(pool.clone());
        let mut job = crate::domain::Job::new("Test Upsert New");
        job.source = Some("greenhouse".to_string());
        job.source_id = Some(format!("test-upsert-new-{}", uuid::Uuid::new_v4()));

        let is_new = repo.upsert_discovered(&job).await.unwrap();
        assert!(is_new, "first insert should be new");

        let _ = sqlx::query("DELETE FROM jobs WHERE id = $1")
            .bind(job.id)
            .execute(&pool)
            .await;
    }

    #[tokio::test]
    async fn upsert_discovered_returns_false_for_update() {
        let url = match std::env::var("DATABASE_URL") {
            Ok(u) => u,
            Err(_) => return,
        };
        let pool = sqlx::PgPool::connect(&url).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        let repo = crate::repositories::JobRepository::new(pool.clone());
        let source_id = format!("test-upsert-update-{}", uuid::Uuid::new_v4());
        let mut job = crate::domain::Job::new("Test Upsert Update");
        job.source = Some("greenhouse".to_string());
        job.source_id = Some(source_id.clone());

        let is_new = repo.upsert_discovered(&job).await.unwrap();
        assert!(is_new, "first insert should be new");

        let mut job2 = crate::domain::Job::new("Test Upsert Update V2");
        job2.source = Some("greenhouse".to_string());
        job2.source_id = Some(source_id);

        let is_new2 = repo.upsert_discovered(&job2).await.unwrap();
        assert!(!is_new2, "second upsert should not be new");

        let _ = sqlx::query("DELETE FROM jobs WHERE id = $1 OR id = $2")
            .bind(job.id)
            .bind(job2.id)
            .execute(&pool)
            .await;
    }
}
