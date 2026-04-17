use sqlx::PgPool;

use crate::domain::{CompanyId, Contact, ContactId};
use crate::error::{CoreError, Result};

use super::Pagination;

pub struct ContactRepository {
    pool: PgPool,
}

impl ContactRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, contact: &Contact) -> Result<()> {
        sqlx::query(
            "INSERT INTO contacts (id, name, role, email, linkedin_url, company_id, current_company, source, relationship, notes)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(contact.id)
        .bind(&contact.name)
        .bind(&contact.role)
        .bind(&contact.email)
        .bind(&contact.linkedin_url)
        .bind(contact.company_id)
        .bind(&contact.current_company)
        .bind(&contact.source)
        .bind(&contact.relationship)
        .bind(&contact.notes)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn upsert_by_email(&self, contact: &Contact) -> Result<bool> {
        let result = sqlx::query(
            "INSERT INTO contacts (id, name, role, email, linkedin_url, company_id, current_company, source, relationship, notes)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (email) WHERE email IS NOT NULL
             DO UPDATE SET name = EXCLUDED.name, role = EXCLUDED.role,
                           linkedin_url = EXCLUDED.linkedin_url,
                           current_company = EXCLUDED.current_company,
                           source = EXCLUDED.source,
                           updated_at = now()
             RETURNING (xmax = 0) AS is_new",
        )
        .bind(contact.id)
        .bind(&contact.name)
        .bind(&contact.role)
        .bind(&contact.email)
        .bind(&contact.linkedin_url)
        .bind(contact.company_id)
        .bind(&contact.current_company)
        .bind(&contact.source)
        .bind(&contact.relationship)
        .bind(&contact.notes)
        .fetch_one(&self.pool)
        .await;

        match result {
            Ok(row) => {
                use sqlx::Row;
                Ok(row.get::<bool, _>("is_new"))
            }
            Err(e) => Err(e.into()),
        }
    }

    pub async fn find_by_id(&self, id: &ContactId) -> Result<Option<Contact>> {
        let row = sqlx::query_as::<_, ContactRow>(
            "SELECT id, name, role, email, linkedin_url, company_id, current_company, source, relationship, notes
             FROM contacts WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(Into::into))
    }

    pub async fn list(&self, pagination: &Pagination) -> Result<Vec<Contact>> {
        let rows = sqlx::query_as::<_, ContactRow>(
            "SELECT id, name, role, email, linkedin_url, company_id, current_company, source, relationship, notes
             FROM contacts ORDER BY name ASC LIMIT $1 OFFSET $2",
        )
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn find_by_company(&self, company_name: &str) -> Result<Vec<Contact>> {
        let rows = sqlx::query_as::<_, ContactRow>(
            "SELECT id, name, role, email, linkedin_url, company_id, current_company, source, relationship, notes
             FROM contacts WHERE LOWER(current_company) = LOWER($1) ORDER BY name ASC",
        )
        .bind(company_name)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn update(&self, contact: &Contact) -> Result<()> {
        let result = sqlx::query(
            "UPDATE contacts SET name = $1, role = $2, email = $3, linkedin_url = $4,
             company_id = $5, current_company = $6, source = $7, relationship = $8, notes = $9, updated_at = now()
             WHERE id = $10",
        )
        .bind(&contact.name)
        .bind(&contact.role)
        .bind(&contact.email)
        .bind(&contact.linkedin_url)
        .bind(contact.company_id)
        .bind(&contact.current_company)
        .bind(&contact.source)
        .bind(&contact.relationship)
        .bind(&contact.notes)
        .bind(contact.id)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(CoreError::NotFound {
                entity: "Contact",
                id: contact.id.to_string(),
            });
        }
        Ok(())
    }

    pub async fn delete(&self, id: &ContactId) -> Result<()> {
        sqlx::query("DELETE FROM contacts WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct ContactRow {
    id: ContactId,
    name: String,
    role: Option<String>,
    email: Option<String>,
    linkedin_url: Option<String>,
    company_id: Option<CompanyId>,
    current_company: Option<String>,
    source: Option<String>,
    relationship: Option<String>,
    notes: Option<String>,
}

impl From<ContactRow> for Contact {
    fn from(row: ContactRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            role: row.role,
            email: row.email,
            linkedin_url: row.linkedin_url,
            company_id: row.company_id,
            current_company: row.current_company,
            source: row.source,
            relationship: row.relationship,
            notes: row.notes,
        }
    }
}
