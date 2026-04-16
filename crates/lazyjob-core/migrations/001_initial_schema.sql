-- LazyJob initial schema (PostgreSQL)
-- All IDs are UUID, timestamps are TIMESTAMPTZ, arrays are TEXT[]

CREATE TABLE companies (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    website TEXT,
    industry TEXT,
    size TEXT,
    tech_stack TEXT[] NOT NULL DEFAULT '{}',
    culture_keywords TEXT[] NOT NULL DEFAULT '{}',
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE jobs (
    id UUID PRIMARY KEY,
    title TEXT NOT NULL,
    company_id UUID REFERENCES companies(id) ON DELETE SET NULL,
    company_name TEXT,
    location TEXT,
    url TEXT,
    description TEXT,
    salary_min BIGINT,
    salary_max BIGINT,
    source TEXT,
    source_id TEXT,
    match_score DOUBLE PRECISION,
    ghost_score DOUBLE PRECISION,
    discovered_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE applications (
    id UUID PRIMARY KEY,
    job_id UUID NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    stage TEXT NOT NULL DEFAULT 'interested',
    submitted_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    resume_version TEXT,
    cover_letter_version TEXT,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE application_transitions (
    id UUID PRIMARY KEY,
    application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    from_stage TEXT NOT NULL,
    to_stage TEXT NOT NULL,
    transitioned_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    notes TEXT
);

CREATE TABLE contacts (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    role TEXT,
    email TEXT,
    linkedin_url TEXT,
    company_id UUID REFERENCES companies(id) ON DELETE SET NULL,
    relationship TEXT,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE interviews (
    id UUID PRIMARY KEY,
    application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    interview_type TEXT NOT NULL,
    scheduled_at TIMESTAMPTZ,
    location TEXT,
    notes TEXT,
    completed BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE offers (
    id UUID PRIMARY KEY,
    application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    salary BIGINT,
    equity TEXT,
    benefits TEXT,
    deadline TIMESTAMPTZ,
    accepted BOOLEAN,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE life_sheet_items (
    id UUID PRIMARY KEY,
    section TEXT NOT NULL,
    key TEXT NOT NULL,
    value JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(section, key)
);

CREATE TABLE token_usage_log (
    id UUID PRIMARY KEY,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cost_microdollars BIGINT NOT NULL DEFAULT 0,
    operation TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE ralph_loop_runs (
    id UUID PRIMARY KEY,
    loop_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending', 'running', 'done', 'failed', 'cancelled')),
    params_json JSONB,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Indexes
CREATE INDEX idx_jobs_company_id ON jobs(company_id);
CREATE INDEX idx_jobs_source ON jobs(source, source_id);
CREATE INDEX idx_jobs_discovered_at ON jobs(discovered_at);
CREATE INDEX idx_applications_job_id ON applications(job_id);
CREATE INDEX idx_applications_stage ON applications(stage);
CREATE INDEX idx_application_transitions_app_id ON application_transitions(application_id);
CREATE INDEX idx_contacts_company_id ON contacts(company_id);
CREATE INDEX idx_interviews_application_id ON interviews(application_id);
CREATE INDEX idx_offers_application_id ON offers(application_id);
CREATE INDEX idx_life_sheet_section ON life_sheet_items(section);
CREATE INDEX idx_token_usage_created ON token_usage_log(created_at);
CREATE INDEX idx_ralph_runs_status ON ralph_loop_runs(status);
