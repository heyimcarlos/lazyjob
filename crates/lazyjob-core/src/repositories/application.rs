use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::{Application, ApplicationId, ApplicationStage, JobId, StageTransition};
use crate::error::{CoreError, Result};

use super::Pagination;

pub struct ApplicationRepository {
    pool: PgPool,
}

impl ApplicationRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, app: &Application) -> Result<()> {
        sqlx::query(
            "INSERT INTO applications (id, job_id, stage, submitted_at, updated_at,
             resume_version, cover_letter_version, notes)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(app.id)
        .bind(app.job_id)
        .bind(app.stage.as_str())
        .bind(app.submitted_at)
        .bind(app.updated_at)
        .bind(&app.resume_version)
        .bind(&app.cover_letter_version)
        .bind(&app.notes)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn find_by_id(&self, id: &ApplicationId) -> Result<Option<Application>> {
        let row = sqlx::query_as::<_, ApplicationRow>(
            "SELECT id, job_id, stage, submitted_at, updated_at,
             resume_version, cover_letter_version, notes
             FROM applications WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(TryInto::try_into).transpose()
    }

    pub async fn list(&self, pagination: &Pagination) -> Result<Vec<Application>> {
        let rows = sqlx::query_as::<_, ApplicationRow>(
            "SELECT id, job_id, stage, submitted_at, updated_at,
             resume_version, cover_letter_version, notes
             FROM applications ORDER BY updated_at DESC LIMIT $1 OFFSET $2",
        )
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn update(&self, app: &Application) -> Result<()> {
        let result = sqlx::query(
            "UPDATE applications SET job_id = $1, stage = $2, submitted_at = $3,
             updated_at = now(), resume_version = $4, cover_letter_version = $5, notes = $6
             WHERE id = $7",
        )
        .bind(app.job_id)
        .bind(app.stage.as_str())
        .bind(app.submitted_at)
        .bind(&app.resume_version)
        .bind(&app.cover_letter_version)
        .bind(&app.notes)
        .bind(app.id)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(CoreError::NotFound {
                entity: "Application",
                id: app.id.to_string(),
            });
        }
        Ok(())
    }

    pub async fn delete(&self, id: &ApplicationId) -> Result<()> {
        sqlx::query("DELETE FROM applications WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn transition_stage(
        &self,
        id: &ApplicationId,
        next_stage: ApplicationStage,
        reason: Option<&str>,
    ) -> Result<StageTransition> {
        let app = self
            .find_by_id(id)
            .await?
            .ok_or_else(|| CoreError::NotFound {
                entity: "Application",
                id: id.to_string(),
            })?;

        if !app.stage.can_transition_to(next_stage) {
            return Err(CoreError::Validation(format!(
                "invalid stage transition: {} -> {}",
                app.stage.as_str(),
                next_stage.as_str()
            )));
        }

        let transition_id = Uuid::new_v4();
        let now = Utc::now();

        let mut tx = self.pool.begin().await?;

        sqlx::query("UPDATE applications SET stage = $1, updated_at = now() WHERE id = $2")
            .bind(next_stage.as_str())
            .bind(id)
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            "INSERT INTO application_transitions (id, application_id, from_stage, to_stage, transitioned_at, notes)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(transition_id)
        .bind(id)
        .bind(app.stage.as_str())
        .bind(next_stage.as_str())
        .bind(now)
        .bind(reason)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(StageTransition {
            id: transition_id,
            application_id: *id,
            from_stage: app.stage,
            to_stage: next_stage,
            transitioned_at: now,
            notes: reason.map(String::from),
        })
    }

    pub async fn transition_history(&self, id: &ApplicationId) -> Result<Vec<StageTransition>> {
        let rows = sqlx::query_as::<_, TransitionRow>(
            "SELECT id, application_id, from_stage, to_stage, transitioned_at, notes
             FROM application_transitions
             WHERE application_id = $1
             ORDER BY transitioned_at ASC",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(TryInto::try_into).collect()
    }
}

#[derive(sqlx::FromRow)]
struct ApplicationRow {
    id: ApplicationId,
    job_id: JobId,
    stage: String,
    submitted_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
    resume_version: Option<String>,
    cover_letter_version: Option<String>,
    notes: Option<String>,
}

impl TryFrom<ApplicationRow> for Application {
    type Error = CoreError;

    fn try_from(row: ApplicationRow) -> Result<Self> {
        let stage: ApplicationStage = row.stage.parse().map_err(|e: String| CoreError::Parse(e))?;
        Ok(Self {
            id: row.id,
            job_id: row.job_id,
            stage,
            submitted_at: row.submitted_at,
            updated_at: row.updated_at,
            resume_version: row.resume_version,
            cover_letter_version: row.cover_letter_version,
            notes: row.notes,
        })
    }
}

#[derive(sqlx::FromRow)]
struct TransitionRow {
    id: Uuid,
    application_id: ApplicationId,
    from_stage: String,
    to_stage: String,
    transitioned_at: DateTime<Utc>,
    notes: Option<String>,
}

impl TryFrom<TransitionRow> for StageTransition {
    type Error = CoreError;

    fn try_from(row: TransitionRow) -> Result<Self> {
        let from_stage: ApplicationStage = row
            .from_stage
            .parse()
            .map_err(|e: String| CoreError::Parse(e))?;
        let to_stage: ApplicationStage = row
            .to_stage
            .parse()
            .map_err(|e: String| CoreError::Parse(e))?;
        Ok(Self {
            id: row.id,
            application_id: row.application_id,
            from_stage,
            to_stage,
            transitioned_at: row.transitioned_at,
            notes: row.notes,
        })
    }
}
