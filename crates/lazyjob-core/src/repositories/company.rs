use sqlx::PgPool;

use crate::domain::{Company, CompanyId};
use crate::error::{CoreError, Result};

use super::Pagination;

pub struct CompanyRepository {
    pool: PgPool,
}

impl CompanyRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, company: &Company) -> Result<()> {
        sqlx::query(
            "INSERT INTO companies (id, name, website, industry, size, tech_stack, culture_keywords, notes)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(company.id)
        .bind(&company.name)
        .bind(&company.website)
        .bind(&company.industry)
        .bind(&company.size)
        .bind(&company.tech_stack)
        .bind(&company.culture_keywords)
        .bind(&company.notes)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn find_by_id(&self, id: &CompanyId) -> Result<Option<Company>> {
        let row = sqlx::query_as::<_, CompanyRow>(
            "SELECT id, name, website, industry, size, tech_stack, culture_keywords, notes
             FROM companies WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(Into::into))
    }

    pub async fn list(&self, pagination: &Pagination) -> Result<Vec<Company>> {
        let rows = sqlx::query_as::<_, CompanyRow>(
            "SELECT id, name, website, industry, size, tech_stack, culture_keywords, notes
             FROM companies ORDER BY name ASC LIMIT $1 OFFSET $2",
        )
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn update(&self, company: &Company) -> Result<()> {
        let result = sqlx::query(
            "UPDATE companies SET name = $1, website = $2, industry = $3, size = $4,
             tech_stack = $5, culture_keywords = $6, notes = $7, updated_at = now()
             WHERE id = $8",
        )
        .bind(&company.name)
        .bind(&company.website)
        .bind(&company.industry)
        .bind(&company.size)
        .bind(&company.tech_stack)
        .bind(&company.culture_keywords)
        .bind(&company.notes)
        .bind(company.id)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(CoreError::NotFound {
                entity: "Company",
                id: company.id.to_string(),
            });
        }
        Ok(())
    }

    pub async fn delete(&self, id: &CompanyId) -> Result<()> {
        sqlx::query("DELETE FROM companies WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct CompanyRow {
    id: CompanyId,
    name: String,
    website: Option<String>,
    industry: Option<String>,
    size: Option<String>,
    tech_stack: Vec<String>,
    culture_keywords: Vec<String>,
    notes: Option<String>,
}

impl From<CompanyRow> for Company {
    fn from(row: CompanyRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            website: row.website,
            industry: row.industry,
            size: row.size,
            tech_stack: row.tech_stack,
            culture_keywords: row.culture_keywords,
            notes: row.notes,
        }
    }
}
