CREATE TABLE resume_versions (
    id UUID PRIMARY KEY,
    job_id UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    application_id UUID REFERENCES applications(id) ON DELETE SET NULL,
    label TEXT NOT NULL DEFAULT 'v1',
    content_json JSONB NOT NULL,
    gap_report_json JSONB NOT NULL,
    fabrication_report_json JSONB NOT NULL,
    options_json JSONB NOT NULL,
    is_submitted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_resume_versions_job ON resume_versions(job_id);
CREATE INDEX idx_resume_versions_application ON resume_versions(application_id);
CREATE INDEX idx_resume_versions_submitted ON resume_versions(job_id, is_submitted);
