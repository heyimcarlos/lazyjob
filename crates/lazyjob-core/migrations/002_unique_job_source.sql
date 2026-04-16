-- Partial unique index on (source, source_id) for ON CONFLICT upsert deduplication.
-- NULL values are excluded so manually-entered jobs bypass the constraint.
CREATE UNIQUE INDEX idx_jobs_source_id_unique
    ON jobs(source, source_id)
    WHERE source IS NOT NULL AND source_id IS NOT NULL;
