CREATE TABLE outreach_drafts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    contact_id UUID NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    job_id UUID REFERENCES jobs(id) ON DELETE SET NULL,
    tone TEXT NOT NULL DEFAULT 'professional',
    subject TEXT,
    body TEXT NOT NULL,
    fabrication_warnings JSONB NOT NULL DEFAULT '[]'::jsonb,
    char_count INTEGER NOT NULL DEFAULT 0,
    word_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_outreach_drafts_contact ON outreach_drafts(contact_id);
CREATE INDEX idx_outreach_drafts_job ON outreach_drafts(job_id) WHERE job_id IS NOT NULL;
