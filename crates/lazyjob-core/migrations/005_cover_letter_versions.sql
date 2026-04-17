CREATE TABLE IF NOT EXISTS cover_letter_versions (
    id              UUID NOT NULL PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id          UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    application_id  UUID REFERENCES applications(id) ON DELETE SET NULL,
    version         INTEGER NOT NULL DEFAULT 1,
    template        TEXT NOT NULL DEFAULT 'standard_professional',
    content         TEXT NOT NULL,
    plain_text      TEXT NOT NULL,
    key_points      JSONB NOT NULL DEFAULT '[]',
    tone            TEXT NOT NULL DEFAULT 'professional',
    length          TEXT NOT NULL DEFAULT 'standard',
    options_json    JSONB NOT NULL DEFAULT '{}',
    diff_from_previous TEXT,
    is_submitted    BOOLEAN NOT NULL DEFAULT FALSE,
    label           TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_clv_job_id ON cover_letter_versions(job_id);
CREATE INDEX IF NOT EXISTS idx_clv_app_id ON cover_letter_versions(application_id);
CREATE INDEX IF NOT EXISTS idx_clv_job_version ON cover_letter_versions(job_id, version DESC);
