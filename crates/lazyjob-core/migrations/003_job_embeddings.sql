CREATE TABLE job_embeddings (
    job_id     UUID PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    embedding  BYTEA NOT NULL,
    embedded_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
