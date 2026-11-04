ALTER TABLE agent_runs
    ADD COLUMN provider_name TEXT NOT NULL DEFAULT 'unknown',
    ADD COLUMN data_policy TEXT NOT NULL DEFAULT 'local_only';
