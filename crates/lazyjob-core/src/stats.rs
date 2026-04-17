use std::collections::HashMap;

use sqlx::PgPool;

use crate::domain::{ApplicationId, ApplicationStage};
use crate::error::Result;

#[derive(Debug, Clone, Default)]
pub struct DashboardStats {
    pub total_jobs: i64,
    pub applied_this_week: i64,
    pub in_pipeline: i64,
    pub interviewing: i64,
    pub stage_counts: HashMap<ApplicationStage, i64>,
}

#[derive(Debug, Clone)]
pub struct StaleApplication {
    pub application_id: ApplicationId,
    pub job_title: String,
    pub company: String,
    pub days_stale: i64,
}

pub async fn compute_dashboard_stats(pool: &PgPool) -> Result<DashboardStats> {
    let total_jobs: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM jobs")
        .fetch_one(pool)
        .await?;

    let applied_this_week: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM applications
         WHERE stage != 'interested'
         AND updated_at >= now() - interval '7 days'",
    )
    .fetch_one(pool)
    .await?;

    let stage_rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT stage, COUNT(*) FROM applications GROUP BY stage")
            .fetch_all(pool)
            .await?;

    let mut stage_counts = HashMap::new();
    let mut in_pipeline: i64 = 0;
    let mut interviewing: i64 = 0;

    for (stage_str, count) in &stage_rows {
        if let Ok(stage) = stage_str.parse::<ApplicationStage>() {
            stage_counts.insert(stage, *count);
            if !stage.is_terminal() && stage != ApplicationStage::Interested {
                in_pipeline += count;
            }
            if matches!(
                stage,
                ApplicationStage::PhoneScreen
                    | ApplicationStage::Technical
                    | ApplicationStage::Onsite
            ) {
                interviewing += count;
            }
        }
    }

    Ok(DashboardStats {
        total_jobs: total_jobs.0,
        applied_this_week: applied_this_week.0,
        in_pipeline,
        interviewing,
        stage_counts,
    })
}

pub async fn find_stale_applications(pool: &PgPool) -> Result<Vec<StaleApplication>> {
    let rows: Vec<(ApplicationId, String, Option<String>, f64)> = sqlx::query_as(
        "SELECT a.id, j.title, j.company_name,
                (EXTRACT(EPOCH FROM (now() - a.updated_at)) / 86400.0)::FLOAT8 AS days_stale
         FROM applications a
         JOIN jobs j ON a.job_id = j.id
         WHERE a.stage NOT IN ('accepted', 'rejected', 'withdrawn')
         AND a.updated_at < now() - interval '14 days'
         ORDER BY a.updated_at ASC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(application_id, job_title, company, days)| StaleApplication {
                application_id,
                job_title,
                company: company.unwrap_or_default(),
                days_stale: days as i64,
            },
        )
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Application, Job};
    use crate::repositories::{ApplicationRepository, JobRepository};
    use crate::test_db::TestDb;

    #[test]
    fn default_stats_are_zero() {
        let stats = DashboardStats::default();
        assert_eq!(stats.total_jobs, 0);
        assert_eq!(stats.applied_this_week, 0);
        assert_eq!(stats.in_pipeline, 0);
        assert_eq!(stats.interviewing, 0);
        assert!(stats.stage_counts.is_empty());
    }

    #[test]
    fn stale_application_fields() {
        let stale = StaleApplication {
            application_id: ApplicationId::new(),
            job_title: "Rust Dev".into(),
            company: "TestCo".into(),
            days_stale: 21,
        };
        assert_eq!(stale.job_title, "Rust Dev");
        assert_eq!(stale.days_stale, 21);
    }

    #[tokio::test]
    async fn compute_stats_empty_db() {
        let db = TestDb::spawn().await;
        let stats = compute_dashboard_stats(db.pool()).await.unwrap();
        assert_eq!(stats.total_jobs, 0);
        assert_eq!(stats.applied_this_week, 0);
        assert_eq!(stats.in_pipeline, 0);
        assert_eq!(stats.interviewing, 0);
    }

    #[tokio::test]
    async fn compute_stats_with_data() {
        let db = TestDb::spawn().await;
        let job_repo = JobRepository::new(db.pool().clone());
        let app_repo = ApplicationRepository::new(db.pool().clone());

        let job1 = Job::new("Job A");
        let job2 = Job::new("Job B");
        let job3 = Job::new("Job C");
        job_repo.insert(&job1).await.unwrap();
        job_repo.insert(&job2).await.unwrap();
        job_repo.insert(&job3).await.unwrap();

        let mut app1 = Application::new(job1.id);
        app1.stage = ApplicationStage::Applied;
        app_repo.insert(&app1).await.unwrap();

        let mut app2 = Application::new(job2.id);
        app2.stage = ApplicationStage::Technical;
        app_repo.insert(&app2).await.unwrap();

        let mut app3 = Application::new(job3.id);
        app3.stage = ApplicationStage::Rejected;
        app_repo.insert(&app3).await.unwrap();

        let stats = compute_dashboard_stats(db.pool()).await.unwrap();
        assert_eq!(stats.total_jobs, 3);
        assert_eq!(stats.in_pipeline, 2); // Applied + Technical
        assert_eq!(stats.interviewing, 1); // Technical only
        assert_eq!(stats.stage_counts.get(&ApplicationStage::Applied), Some(&1));
        assert_eq!(
            stats.stage_counts.get(&ApplicationStage::Technical),
            Some(&1)
        );
        assert_eq!(
            stats.stage_counts.get(&ApplicationStage::Rejected),
            Some(&1)
        );
    }

    #[tokio::test]
    async fn find_stale_empty_db() {
        let db = TestDb::spawn().await;
        let stale = find_stale_applications(db.pool()).await.unwrap();
        assert!(stale.is_empty());
    }

    #[tokio::test]
    async fn find_stale_ignores_terminal() {
        let db = TestDb::spawn().await;
        let job_repo = JobRepository::new(db.pool().clone());

        let job = Job::new("Old Job");
        job_repo.insert(&job).await.unwrap();

        // Insert a rejected application with old updated_at
        sqlx::query(
            "INSERT INTO applications (id, job_id, stage, updated_at)
             VALUES ($1, $2, 'rejected', now() - interval '30 days')",
        )
        .bind(ApplicationId::new())
        .bind(job.id)
        .execute(db.pool())
        .await
        .unwrap();

        let stale = find_stale_applications(db.pool()).await.unwrap();
        assert!(stale.is_empty());
    }

    #[tokio::test]
    async fn find_stale_returns_old_active_apps() {
        let db = TestDb::spawn().await;
        let job_repo = JobRepository::new(db.pool().clone());

        let mut job = Job::new("Stale Job");
        job.company_name = Some("StaleCo".into());
        job_repo.insert(&job).await.unwrap();

        let app_id = ApplicationId::new();
        sqlx::query(
            "INSERT INTO applications (id, job_id, stage, updated_at)
             VALUES ($1, $2, 'applied', now() - interval '20 days')",
        )
        .bind(app_id)
        .bind(job.id)
        .execute(db.pool())
        .await
        .unwrap();

        let stale = find_stale_applications(db.pool()).await.unwrap();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].job_title, "Stale Job");
        assert_eq!(stale[0].company, "StaleCo");
        assert!(stale[0].days_stale >= 19);
    }

    #[tokio::test]
    async fn find_stale_ignores_recent_apps() {
        let db = TestDb::spawn().await;
        let job_repo = JobRepository::new(db.pool().clone());
        let app_repo = ApplicationRepository::new(db.pool().clone());

        let job = Job::new("Fresh Job");
        job_repo.insert(&job).await.unwrap();

        let mut app = Application::new(job.id);
        app.stage = ApplicationStage::Applied;
        app_repo.insert(&app).await.unwrap();

        let stale = find_stale_applications(db.pool()).await.unwrap();
        assert!(stale.is_empty());
    }
}
