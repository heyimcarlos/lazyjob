ALTER TABLE contacts ADD COLUMN IF NOT EXISTS current_company TEXT;
ALTER TABLE contacts ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'manual';

CREATE UNIQUE INDEX IF NOT EXISTS idx_contacts_email_unique ON contacts(email) WHERE email IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_contacts_current_company ON contacts(LOWER(current_company));
