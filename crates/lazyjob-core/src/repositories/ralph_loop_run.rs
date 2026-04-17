use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{CoreError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RalphLoopRunStatus {
    Pending,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl RalphLoopRunStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl FromStr for RalphLoopRunStatus {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(CoreError::Parse(format!(
                "unknown ralph loop run status: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RalphLoopRun {
    pub id: Uuid,
    pub loop_type: String,
    pub status: RalphLoopRunStatus,
    pub params_json: Option<serde_json::Value>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl RalphLoopRun {
    pub fn new(loop_type: &str) -> Self {
        Self {
            id: Uuid::new_v4(),
            loop_type: loop_type.to_string(),
            status: RalphLoopRunStatus::Pending,
            params_json: None,
            started_at: None,
            finished_at: None,
            created_at: Utc::now(),
        }
    }
}

pub struct RalphLoopRunRepository {
    pool: PgPool,
}

impl RalphLoopRunRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert_run(&self, run: &RalphLoopRun) -> Result<()> {
        sqlx::query(
            "INSERT INTO ralph_loop_runs (id, loop_type, status, params_json, started_at, finished_at, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(run.id)
        .bind(&run.loop_type)
        .bind(run.status.as_str())
        .bind(&run.params_json)
        .bind(run.started_at)
        .bind(run.finished_at)
        .bind(run.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_status(&self, id: &Uuid, status: RalphLoopRunStatus) -> Result<()> {
        let result = sqlx::query("UPDATE ralph_loop_runs SET status = $1 WHERE id = $2")
            .bind(status.as_str())
            .bind(id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(CoreError::NotFound {
                entity: "RalphLoopRun",
                id: id.to_string(),
            });
        }
        Ok(())
    }

    pub async fn list_pending(&self) -> Result<Vec<RalphLoopRun>> {
        let rows = sqlx::query_as::<_, RalphLoopRunRow>(
            "SELECT id, loop_type, status, params_json, started_at, finished_at, created_at
             FROM ralph_loop_runs WHERE status = 'pending' ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(TryFrom::try_from).collect()
    }

    pub async fn recover_pending(&self) -> Result<usize> {
        let result = sqlx::query(
            "UPDATE ralph_loop_runs SET status = 'failed'
             WHERE status = 'running'
               AND started_at < now() - interval '30 seconds'",
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() as usize)
    }
}

#[derive(sqlx::FromRow)]
struct RalphLoopRunRow {
    id: Uuid,
    loop_type: String,
    status: String,
    params_json: Option<serde_json::Value>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

impl TryFrom<RalphLoopRunRow> for RalphLoopRun {
    type Error = CoreError;

    fn try_from(row: RalphLoopRunRow) -> Result<Self> {
        Ok(Self {
            id: row.id,
            loop_type: row.loop_type,
            status: row.status.parse()?,
            params_json: row.params_json,
            started_at: row.started_at,
            finished_at: row.finished_at,
            created_at: row.created_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ralph_loop_run_status_as_str() {
        assert_eq!(RalphLoopRunStatus::Pending.as_str(), "pending");
        assert_eq!(RalphLoopRunStatus::Running.as_str(), "running");
        assert_eq!(RalphLoopRunStatus::Done.as_str(), "done");
        assert_eq!(RalphLoopRunStatus::Failed.as_str(), "failed");
        assert_eq!(RalphLoopRunStatus::Cancelled.as_str(), "cancelled");
    }

    #[test]
    fn ralph_loop_run_status_from_str() {
        assert_eq!(
            "pending".parse::<RalphLoopRunStatus>().unwrap(),
            RalphLoopRunStatus::Pending
        );
        assert_eq!(
            "running".parse::<RalphLoopRunStatus>().unwrap(),
            RalphLoopRunStatus::Running
        );
        assert_eq!(
            "done".parse::<RalphLoopRunStatus>().unwrap(),
            RalphLoopRunStatus::Done
        );
        assert_eq!(
            "failed".parse::<RalphLoopRunStatus>().unwrap(),
            RalphLoopRunStatus::Failed
        );
        assert_eq!(
            "cancelled".parse::<RalphLoopRunStatus>().unwrap(),
            RalphLoopRunStatus::Cancelled
        );
    }

    #[test]
    fn ralph_loop_run_status_from_str_invalid() {
        let result = "unknown".parse::<RalphLoopRunStatus>();
        assert!(result.is_err());
    }

    #[test]
    fn ralph_loop_run_new_has_pending_status() {
        let run = RalphLoopRun::new("job_discovery");
        assert_eq!(run.status, RalphLoopRunStatus::Pending);
        assert_eq!(run.loop_type, "job_discovery");
        assert!(run.started_at.is_none());
        assert!(run.finished_at.is_none());
        assert!(run.params_json.is_none());
    }

    #[test]
    fn status_round_trip_via_as_str() {
        let statuses = [
            RalphLoopRunStatus::Pending,
            RalphLoopRunStatus::Running,
            RalphLoopRunStatus::Done,
            RalphLoopRunStatus::Failed,
            RalphLoopRunStatus::Cancelled,
        ];
        for status in &statuses {
            let parsed: RalphLoopRunStatus = status.as_str().parse().unwrap();
            assert_eq!(&parsed, status);
        }
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::test_db::TestDb;

    #[tokio::test]
    async fn insert_and_list_pending() {
        let db = TestDb::spawn().await;
        let repo = RalphLoopRunRepository::new(db.pool().clone());

        let run = RalphLoopRun::new("job_discovery");
        repo.insert_run(&run).await.unwrap();

        let pending = repo.list_pending().await.unwrap();
        assert!(pending.iter().any(|r| r.id == run.id));
        assert_eq!(
            pending.iter().find(|r| r.id == run.id).unwrap().loop_type,
            "job_discovery"
        );
    }

    #[tokio::test]
    async fn update_status_changes_status() {
        let db = TestDb::spawn().await;
        let repo = RalphLoopRunRepository::new(db.pool().clone());

        let run = RalphLoopRun::new("resume_tailor");
        repo.insert_run(&run).await.unwrap();

        repo.update_status(&run.id, RalphLoopRunStatus::Running)
            .await
            .unwrap();

        let row: (String,) = sqlx::query_as("SELECT status FROM ralph_loop_runs WHERE id = $1")
            .bind(run.id)
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(row.0, "running");
    }

    #[tokio::test]
    async fn update_status_not_found() {
        let db = TestDb::spawn().await;
        let repo = RalphLoopRunRepository::new(db.pool().clone());

        let result = repo
            .update_status(&Uuid::new_v4(), RalphLoopRunStatus::Done)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn recover_pending_marks_stale_running_as_failed() {
        let db = TestDb::spawn().await;
        let repo = RalphLoopRunRepository::new(db.pool().clone());

        let run_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO ralph_loop_runs (id, loop_type, status, started_at, created_at)
             VALUES ($1, 'cover_letter', 'running', now() - interval '5 minutes', now() - interval '5 minutes')",
        )
        .bind(run_id)
        .execute(db.pool())
        .await
        .unwrap();

        let recovered = repo.recover_pending().await.unwrap();
        assert!(recovered >= 1);

        let row: (String,) = sqlx::query_as("SELECT status FROM ralph_loop_runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(row.0, "failed");
    }

    #[tokio::test]
    async fn recover_pending_skips_recent_runs() {
        let db = TestDb::spawn().await;
        let repo = RalphLoopRunRepository::new(db.pool().clone());

        let run_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO ralph_loop_runs (id, loop_type, status, started_at, created_at)
             VALUES ($1, 'job_discovery', 'running', now(), now())",
        )
        .bind(run_id)
        .execute(db.pool())
        .await
        .unwrap();

        repo.recover_pending().await.unwrap();

        let row: (String,) = sqlx::query_as("SELECT status FROM ralph_loop_runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(row.0, "running");
    }

    #[tokio::test]
    async fn recover_pending_ignores_done_runs() {
        let db = TestDb::spawn().await;
        let repo = RalphLoopRunRepository::new(db.pool().clone());

        let run_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO ralph_loop_runs (id, loop_type, status, started_at, finished_at, created_at)
             VALUES ($1, 'interview_prep', 'done', now() - interval '1 hour', now() - interval '30 minutes', now() - interval '1 hour')",
        )
        .bind(run_id)
        .execute(db.pool())
        .await
        .unwrap();

        repo.recover_pending().await.unwrap();

        let row: (String,) = sqlx::query_as("SELECT status FROM ralph_loop_runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(row.0, "done");
    }
}
