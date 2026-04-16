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
            "INSERT INTO contacts (id, name, role, email, linkedin_url, company_id, relationship, notes)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(contact.id)
        .bind(&contact.name)
        .bind(&contact.role)
        .bind(&contact.email)
        .bind(&contact.linkedin_url)
        .bind(contact.company_id)
        .bind(&contact.relationship)
        .bind(&contact.notes)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn find_by_id(&self, id: &ContactId) -> Result<Option<Contact>> {
        let row = sqlx::query_as::<_, ContactRow>(
            "SELECT id, name, role, email, linkedin_url, company_id, relationship, notes
             FROM contacts WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(Into::into))
    }

    pub async fn list(&self, pagination: &Pagination) -> Result<Vec<Contact>> {
        let rows = sqlx::query_as::<_, ContactRow>(
            "SELECT id, name, role, email, linkedin_url, company_id, relationship, notes
             FROM contacts ORDER BY name ASC LIMIT $1 OFFSET $2",
        )
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn update(&self, contact: &Contact) -> Result<()> {
        let result = sqlx::query(
            "UPDATE contacts SET name = $1, role = $2, email = $3, linkedin_url = $4,
             company_id = $5, relationship = $6, notes = $7, updated_at = now()
             WHERE id = $8",
        )
        .bind(&contact.name)
        .bind(&contact.role)
        .bind(&contact.email)
        .bind(&contact.linkedin_url)
        .bind(contact.company_id)
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
            relationship: row.relationship,
            notes: row.notes,
        }
    }
}
