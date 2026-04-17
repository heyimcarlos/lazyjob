use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::Result;

use super::types::{
    FabricationReport, GapReport, ResumeContent, ResumeVersion, ResumeVersionId,
    ResumeVersionSummary, TailoringOptions,
};

pub struct ResumeVersionRepository {
    pool: PgPool,
}

impl ResumeVersionRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn save(&self, version: &ResumeVersion) -> Result<()> {
        let content_json = serde_json::to_value(&version.content)?;
        let gap_json = serde_json::to_value(&version.gap_report)?;
        let fab_json = serde_json::to_value(&version.fabrication_report)?;
        let opts_json = serde_json::to_value(&version.tailoring_options)?;

        sqlx::query(
            "INSERT INTO resume_versions \
             (id, job_id, application_id, label, content_json, gap_report_json, \
              fabrication_report_json, options_json, is_submitted, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(version.id.0)
        .bind(version.job_id)
        .bind(version.application_id)
        .bind(&version.label)
        .bind(content_json)
        .bind(gap_json)
        .bind(fab_json)
        .bind(opts_json)
        .bind(version.is_submitted)
        .bind(version.created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn list_for_job(&self, job_id: &Uuid) -> Result<Vec<ResumeVersionSummary>> {
        let rows = sqlx::query_as::<_, ResumeVersionRow>(
            "SELECT id, label, gap_report_json, is_submitted, created_at \
             FROM resume_versions WHERE job_id = $1 ORDER BY created_at DESC",
        )
        .bind(job_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let match_score = serde_json::from_value::<GapReport>(r.gap_report_json)
                    .map(|g| g.match_score)
                    .unwrap_or(0.0);
                ResumeVersionSummary {
                    id: ResumeVersionId(r.id),
                    label: r.label,
                    match_score,
                    is_submitted: r.is_submitted,
                    created_at: r.created_at,
                }
            })
            .collect())
    }

    pub async fn get(&self, id: &ResumeVersionId) -> Result<Option<ResumeVersion>> {
        let row = sqlx::query_as::<_, FullResumeVersionRow>(
            "SELECT id, job_id, application_id, label, content_json, gap_report_json, \
             fabrication_report_json, options_json, is_submitted, created_at \
             FROM resume_versions WHERE id = $1",
        )
        .bind(id.0)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => {
                let content: ResumeContent = serde_json::from_value(r.content_json)?;
                let gap_report: GapReport = serde_json::from_value(r.gap_report_json)?;
                let fabrication_report: FabricationReport =
                    serde_json::from_value(r.fabrication_report_json)?;
                let tailoring_options: TailoringOptions = serde_json::from_value(r.options_json)?;

                Ok(Some(ResumeVersion {
                    id: ResumeVersionId(r.id),
                    job_id: r.job_id,
                    application_id: r.application_id,
                    content,
                    gap_report,
                    fabrication_report,
                    tailoring_options,
                    created_at: r.created_at,
                    label: r.label,
                    is_submitted: r.is_submitted,
                }))
            }
            None => Ok(None),
        }
    }

    pub async fn mark_submitted(&self, id: &ResumeVersionId) -> Result<()> {
        sqlx::query("UPDATE resume_versions SET is_submitted = TRUE WHERE id = $1")
            .bind(id.0)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn count_for_job(&self, job_id: &Uuid) -> Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM resume_versions WHERE job_id = $1")
            .bind(job_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }
}

#[derive(sqlx::FromRow)]
struct ResumeVersionRow {
    id: Uuid,
    label: String,
    gap_report_json: serde_json::Value,
    is_submitted: bool,
    created_at: chrono::DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct FullResumeVersionRow {
    id: Uuid,
    job_id: Uuid,
    application_id: Option<Uuid>,
    label: String,
    content_json: serde_json::Value,
    gap_report_json: serde_json::Value,
    fabrication_report_json: serde_json::Value,
    options_json: serde_json::Value,
    is_submitted: bool,
    created_at: chrono::DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resume::types::{ResumeContent, SkillsSection};
    use crate::test_db::TestDb;

    fn make_resume_version(job_id: Uuid) -> ResumeVersion {
        ResumeVersion {
            id: ResumeVersionId::new(),
            job_id,
            application_id: None,
            content: ResumeContent {
                summary: "Test summary".into(),
                experience: vec![],
                skills: SkillsSection {
                    primary: vec!["Rust".into()],
                    secondary: vec![],
                },
                education: vec![],
                projects: vec![],
                certifications: vec![],
            },
            gap_report: GapReport {
                matched_skills: vec![],
                missing_required: vec![],
                missing_nice_to_have: vec![],
                match_score: 75.0,
                relevant_experience_order: vec![],
            },
            fabrication_report: FabricationReport {
                items: vec![],
                warnings: vec![],
                errors: vec![],
                is_safe_to_submit: true,
            },
            tailoring_options: TailoringOptions::default(),
            created_at: Utc::now(),
            label: "v1".into(),
            is_submitted: false,
        }
    }

    async fn insert_test_job(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO jobs (id, title, discovered_at) VALUES ($1, $2, now())")
            .bind(id)
            .bind("Test Job")
            .execute(pool)
            .await
            .unwrap();
        id
    }

    #[tokio::test]
    async fn save_and_get() {
        let db = TestDb::spawn().await;
        let repo = ResumeVersionRepository::new(db.pool().clone());

        let job_id = insert_test_job(db.pool()).await;
        let version = make_resume_version(job_id);
        let version_id = version.id.clone();

        repo.save(&version).await.unwrap();

        let loaded = repo.get(&version_id).await.unwrap().unwrap();
        assert_eq!(loaded.label, "v1");
        assert_eq!(loaded.job_id, job_id);
        assert!((loaded.gap_report.match_score - 75.0).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn list_for_job() {
        let db = TestDb::spawn().await;
        let repo = ResumeVersionRepository::new(db.pool().clone());

        let job_id = insert_test_job(db.pool()).await;

        let mut v1 = make_resume_version(job_id);
        v1.label = "v1".into();
        repo.save(&v1).await.unwrap();

        let mut v2 = make_resume_version(job_id);
        v2.label = "v2".into();
        repo.save(&v2).await.unwrap();

        let summaries = repo.list_for_job(&job_id).await.unwrap();
        assert_eq!(summaries.len(), 2);
    }

    #[tokio::test]
    async fn mark_submitted() {
        let db = TestDb::spawn().await;
        let repo = ResumeVersionRepository::new(db.pool().clone());

        let job_id = insert_test_job(db.pool()).await;
        let version = make_resume_version(job_id);
        let version_id = version.id.clone();

        repo.save(&version).await.unwrap();
        repo.mark_submitted(&version_id).await.unwrap();

        let loaded = repo.get(&version_id).await.unwrap().unwrap();
        assert!(loaded.is_submitted);
    }

    #[tokio::test]
    async fn count_for_job() {
        let db = TestDb::spawn().await;
        let repo = ResumeVersionRepository::new(db.pool().clone());

        let job_id = insert_test_job(db.pool()).await;
        assert_eq!(repo.count_for_job(&job_id).await.unwrap(), 0);

        repo.save(&make_resume_version(job_id)).await.unwrap();
        assert_eq!(repo.count_for_job(&job_id).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn get_nonexistent_returns_none() {
        let db = TestDb::spawn().await;
        let repo = ResumeVersionRepository::new(db.pool().clone());
        let result = repo.get(&ResumeVersionId::new()).await.unwrap();
        assert!(result.is_none());
    }
}
