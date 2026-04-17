use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::Result;

use super::types::{
    CoverLetterId, CoverLetterLength, CoverLetterOptions, CoverLetterTemplate, CoverLetterTone,
    CoverLetterVersion, CoverLetterVersionSummary,
};

pub struct CoverLetterRepository {
    pool: PgPool,
}

impl CoverLetterRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn save(&self, version: &CoverLetterVersion) -> Result<()> {
        let key_points = serde_json::to_value(&version.key_points)?;
        let options_json = serde_json::to_value(&version.options)?;

        sqlx::query(
            "INSERT INTO cover_letter_versions \
             (id, job_id, application_id, version, template, content, plain_text, \
              key_points, tone, length, options_json, diff_from_previous, \
              is_submitted, label, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
        )
        .bind(version.id.0)
        .bind(version.job_id)
        .bind(version.application_id)
        .bind(version.version)
        .bind(version.template.as_str())
        .bind(&version.content)
        .bind(&version.plain_text)
        .bind(key_points)
        .bind(version.tone.as_str())
        .bind(version.length.as_str())
        .bind(options_json)
        .bind(&version.diff_from_previous)
        .bind(version.is_submitted)
        .bind(&version.label)
        .bind(version.created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get(&self, id: &CoverLetterId) -> Result<Option<CoverLetterVersion>> {
        let row = sqlx::query_as::<_, FullCoverLetterRow>(
            "SELECT id, job_id, application_id, version, template, content, plain_text, \
             key_points, tone, length, options_json, diff_from_previous, \
             is_submitted, label, created_at \
             FROM cover_letter_versions WHERE id = $1",
        )
        .bind(id.0)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => Ok(Some(row_to_version(r)?)),
            None => Ok(None),
        }
    }

    pub async fn list_for_job(&self, job_id: &Uuid) -> Result<Vec<CoverLetterVersionSummary>> {
        let rows = sqlx::query_as::<_, SummaryCoverLetterRow>(
            "SELECT id, version, template, tone, is_submitted, label, created_at \
             FROM cover_letter_versions WHERE job_id = $1 ORDER BY version DESC",
        )
        .bind(job_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| CoverLetterVersionSummary {
                id: CoverLetterId(r.id),
                version: r.version,
                template: CoverLetterTemplate::from_str_loose(&r.template),
                tone: CoverLetterTone::from_str_loose(&r.tone),
                is_submitted: r.is_submitted,
                label: r.label,
                created_at: r.created_at,
            })
            .collect())
    }

    pub async fn pin_to_application(&self, id: &CoverLetterId, application_id: Uuid) -> Result<()> {
        sqlx::query(
            "UPDATE cover_letter_versions \
             SET application_id = $1, is_submitted = TRUE \
             WHERE id = $2",
        )
        .bind(application_id)
        .bind(id.0)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn count_for_job(&self, job_id: &Uuid) -> Result<i64> {
        let row: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM cover_letter_versions WHERE job_id = $1")
                .bind(job_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(row.0)
    }

    pub async fn latest_for_job(&self, job_id: &Uuid) -> Result<Option<CoverLetterVersion>> {
        let row = sqlx::query_as::<_, FullCoverLetterRow>(
            "SELECT id, job_id, application_id, version, template, content, plain_text, \
             key_points, tone, length, options_json, diff_from_previous, \
             is_submitted, label, created_at \
             FROM cover_letter_versions WHERE job_id = $1 ORDER BY version DESC LIMIT 1",
        )
        .bind(job_id)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => Ok(Some(row_to_version(r)?)),
            None => Ok(None),
        }
    }
}

fn row_to_version(r: FullCoverLetterRow) -> Result<CoverLetterVersion> {
    let key_points: Vec<String> = serde_json::from_value(r.key_points)?;
    let options: CoverLetterOptions = serde_json::from_value(r.options_json)?;

    Ok(CoverLetterVersion {
        id: CoverLetterId(r.id),
        job_id: r.job_id,
        application_id: r.application_id,
        version: r.version,
        template: CoverLetterTemplate::from_str_loose(&r.template),
        content: r.content,
        plain_text: r.plain_text,
        key_points,
        tone: CoverLetterTone::from_str_loose(&r.tone),
        length: CoverLetterLength::from_str_loose(&r.length),
        options,
        diff_from_previous: r.diff_from_previous,
        is_submitted: r.is_submitted,
        label: r.label,
        created_at: r.created_at,
    })
}

#[derive(sqlx::FromRow)]
struct FullCoverLetterRow {
    id: Uuid,
    job_id: Uuid,
    application_id: Option<Uuid>,
    version: i32,
    template: String,
    content: String,
    plain_text: String,
    key_points: serde_json::Value,
    tone: String,
    length: String,
    options_json: serde_json::Value,
    diff_from_previous: Option<String>,
    is_submitted: bool,
    label: Option<String>,
    created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct SummaryCoverLetterRow {
    id: Uuid,
    version: i32,
    template: String,
    tone: String,
    is_submitted: bool,
    label: Option<String>,
    created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_db::TestDb;

    fn make_version(job_id: Uuid, version: i32) -> CoverLetterVersion {
        CoverLetterVersion {
            id: CoverLetterId::new(),
            job_id,
            application_id: None,
            version,
            template: CoverLetterTemplate::StandardProfessional,
            content: "Dear Hiring Manager, I am excited to apply.".into(),
            plain_text: "Dear Hiring Manager, I am excited to apply.".into(),
            key_points: vec!["Excited to apply".into()],
            tone: CoverLetterTone::Professional,
            length: CoverLetterLength::Standard,
            options: CoverLetterOptions::default(),
            diff_from_previous: None,
            is_submitted: false,
            label: Some(format!("v{version}")),
            created_at: Utc::now(),
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
        let repo = CoverLetterRepository::new(db.pool().clone());
        let job_id = insert_test_job(db.pool()).await;

        let version = make_version(job_id, 1);
        let vid = version.id;
        repo.save(&version).await.unwrap();

        let loaded = repo.get(&vid).await.unwrap().unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.job_id, job_id);
        assert_eq!(loaded.template, CoverLetterTemplate::StandardProfessional);
        assert_eq!(loaded.key_points, vec!["Excited to apply"]);
    }

    #[tokio::test]
    async fn list_for_job_ordered_by_version_desc() {
        let db = TestDb::spawn().await;
        let repo = CoverLetterRepository::new(db.pool().clone());
        let job_id = insert_test_job(db.pool()).await;

        repo.save(&make_version(job_id, 1)).await.unwrap();
        repo.save(&make_version(job_id, 2)).await.unwrap();
        repo.save(&make_version(job_id, 3)).await.unwrap();

        let summaries = repo.list_for_job(&job_id).await.unwrap();
        assert_eq!(summaries.len(), 3);
        assert_eq!(summaries[0].version, 3);
        assert_eq!(summaries[1].version, 2);
        assert_eq!(summaries[2].version, 1);
    }

    #[tokio::test]
    async fn pin_to_application_sets_submitted() {
        let db = TestDb::spawn().await;
        let repo = CoverLetterRepository::new(db.pool().clone());
        let job_id = insert_test_job(db.pool()).await;

        let app_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO applications (id, job_id, stage, created_at, updated_at) \
             VALUES ($1, $2, 'applied', now(), now())",
        )
        .bind(app_id)
        .bind(job_id)
        .execute(db.pool())
        .await
        .unwrap();

        let version = make_version(job_id, 1);
        let vid = version.id;
        repo.save(&version).await.unwrap();
        repo.pin_to_application(&vid, app_id).await.unwrap();

        let loaded = repo.get(&vid).await.unwrap().unwrap();
        assert!(loaded.is_submitted);
        assert_eq!(loaded.application_id, Some(app_id));
    }

    #[tokio::test]
    async fn count_for_job_counts_correctly() {
        let db = TestDb::spawn().await;
        let repo = CoverLetterRepository::new(db.pool().clone());
        let job_id = insert_test_job(db.pool()).await;

        assert_eq!(repo.count_for_job(&job_id).await.unwrap(), 0);
        repo.save(&make_version(job_id, 1)).await.unwrap();
        assert_eq!(repo.count_for_job(&job_id).await.unwrap(), 1);
        repo.save(&make_version(job_id, 2)).await.unwrap();
        assert_eq!(repo.count_for_job(&job_id).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn get_nonexistent_returns_none() {
        let db = TestDb::spawn().await;
        let repo = CoverLetterRepository::new(db.pool().clone());
        let result = repo.get(&CoverLetterId::new()).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn latest_for_job_returns_highest_version() {
        let db = TestDb::spawn().await;
        let repo = CoverLetterRepository::new(db.pool().clone());
        let job_id = insert_test_job(db.pool()).await;

        repo.save(&make_version(job_id, 1)).await.unwrap();
        repo.save(&make_version(job_id, 2)).await.unwrap();

        let latest = repo.latest_for_job(&job_id).await.unwrap().unwrap();
        assert_eq!(latest.version, 2);
    }

    #[tokio::test]
    async fn latest_for_job_returns_none_when_empty() {
        let db = TestDb::spawn().await;
        let repo = CoverLetterRepository::new(db.pool().clone());
        let job_id = insert_test_job(db.pool()).await;

        let result = repo.latest_for_job(&job_id).await.unwrap();
        assert!(result.is_none());
    }
}
