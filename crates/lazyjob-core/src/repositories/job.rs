use sqlx::PgPool;

use crate::domain::{Job, JobId};
use crate::error::Result;

use super::Pagination;

pub struct JobRepository {
    pool: PgPool,
}

impl JobRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, job: &Job) -> Result<()> {
        sqlx::query(
            "INSERT INTO jobs (id, title, company_id, company_name, location, url, description,
             salary_min, salary_max, source, source_id, match_score, ghost_score, discovered_at, notes)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
        )
        .bind(job.id)
        .bind(&job.title)
        .bind(job.company_id)
        .bind(&job.company_name)
        .bind(&job.location)
        .bind(&job.url)
        .bind(&job.description)
        .bind(job.salary_min)
        .bind(job.salary_max)
        .bind(&job.source)
        .bind(&job.source_id)
        .bind(job.match_score)
        .bind(job.ghost_score)
        .bind(job.discovered_at)
        .bind(&job.notes)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn find_by_id(&self, id: &JobId) -> Result<Option<Job>> {
        let row = sqlx::query_as::<_, JobRow>(
            "SELECT id, title, company_id, company_name, location, url, description,
             salary_min, salary_max, source, source_id, match_score, ghost_score, discovered_at, notes
             FROM jobs WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(Into::into))
    }

    pub async fn list(&self, pagination: &Pagination) -> Result<Vec<Job>> {
        let rows = sqlx::query_as::<_, JobRow>(
            "SELECT id, title, company_id, company_name, location, url, description,
             salary_min, salary_max, source, source_id, match_score, ghost_score, discovered_at, notes
             FROM jobs ORDER BY discovered_at DESC LIMIT $1 OFFSET $2",
        )
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn update(&self, job: &Job) -> Result<()> {
        let result = sqlx::query(
            "UPDATE jobs SET title = $1, company_id = $2, company_name = $3, location = $4,
             url = $5, description = $6, salary_min = $7, salary_max = $8, source = $9,
             source_id = $10, match_score = $11, ghost_score = $12, notes = $13,
             updated_at = now() WHERE id = $14",
        )
        .bind(&job.title)
        .bind(job.company_id)
        .bind(&job.company_name)
        .bind(&job.location)
        .bind(&job.url)
        .bind(&job.description)
        .bind(job.salary_min)
        .bind(job.salary_max)
        .bind(&job.source)
        .bind(&job.source_id)
        .bind(job.match_score)
        .bind(job.ghost_score)
        .bind(&job.notes)
        .bind(job.id)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(crate::error::CoreError::NotFound {
                entity: "Job",
                id: job.id.to_string(),
            });
        }
        Ok(())
    }

    pub async fn delete(&self, id: &JobId) -> Result<()> {
        sqlx::query("DELETE FROM jobs WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn upsert_discovered(&self, job: &Job) -> Result<bool> {
        let row = sqlx::query_as::<_, (bool,)>(
            "INSERT INTO jobs (id, title, company_id, company_name, location, url, description,
             salary_min, salary_max, source, source_id, match_score, ghost_score, discovered_at, notes)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
             ON CONFLICT (source, source_id) WHERE source IS NOT NULL AND source_id IS NOT NULL
             DO UPDATE SET
                 title = EXCLUDED.title,
                 company_name = EXCLUDED.company_name,
                 location = EXCLUDED.location,
                 url = EXCLUDED.url,
                 description = EXCLUDED.description,
                 updated_at = now()
             RETURNING (xmax = 0) AS is_new",
        )
        .bind(job.id)
        .bind(&job.title)
        .bind(job.company_id)
        .bind(&job.company_name)
        .bind(&job.location)
        .bind(&job.url)
        .bind(&job.description)
        .bind(job.salary_min)
        .bind(job.salary_max)
        .bind(&job.source)
        .bind(&job.source_id)
        .bind(job.match_score)
        .bind(job.ghost_score)
        .bind(job.discovered_at)
        .bind(&job.notes)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }
}

#[derive(sqlx::FromRow)]
struct JobRow {
    id: JobId,
    title: String,
    company_id: Option<CompanyId>,
    company_name: Option<String>,
    location: Option<String>,
    url: Option<String>,
    description: Option<String>,
    salary_min: Option<i64>,
    salary_max: Option<i64>,
    source: Option<String>,
    source_id: Option<String>,
    match_score: Option<f64>,
    ghost_score: Option<f64>,
    discovered_at: chrono::DateTime<chrono::Utc>,
    notes: Option<String>,
}

use crate::domain::CompanyId;

impl From<JobRow> for Job {
    fn from(row: JobRow) -> Self {
        Self {
            id: row.id,
            title: row.title,
            company_id: row.company_id,
            company_name: row.company_name,
            location: row.location,
            url: row.url,
            description: row.description,
            salary_min: row.salary_min,
            salary_max: row.salary_max,
            source: row.source,
            source_id: row.source_id,
            match_score: row.match_score,
            ghost_score: row.ghost_score,
            discovered_at: row.discovered_at,
            notes: row.notes,
        }
    }
}
