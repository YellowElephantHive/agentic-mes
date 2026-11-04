CREATE TABLE action_plan_simulations (
    id UUID PRIMARY KEY,
    plan_id UUID NOT NULL REFERENCES action_plans(id),
    plan_version BIGINT NOT NULL CHECK (plan_version > 0),
    source_snapshot_hash TEXT NOT NULL CHECK (source_snapshot_hash ~ '^[0-9a-f]{64}$'),
    simulation_policy_version TEXT NOT NULL CHECK (
        char_length(simulation_policy_version) BETWEEN 1 AND 64
    ),
    deterministic_seed BIGINT NOT NULL,
    canonical_input_hash TEXT NOT NULL CHECK (canonical_input_hash ~ '^[0-9a-f]{64}$'),
    canonical_output_hash TEXT NOT NULL CHECK (canonical_output_hash ~ '^[0-9a-f]{64}$'),
    affected_resources JSONB NOT NULL CHECK (pg_column_size(affected_resources) <= 16384),
    precondition_findings TEXT[] NOT NULL DEFAULT '{}'
        CHECK (cardinality(precondition_findings) <= 32),
    missing_data_findings TEXT[] NOT NULL DEFAULT '{}'
        CHECK (cardinality(missing_data_findings) <= 32),
    required_human_roles TEXT[] NOT NULL DEFAULT '{}'
        CHECK (cardinality(required_human_roles) <= 8),
    impact_preview TEXT[] NOT NULL DEFAULT '{}'
        CHECK (cardinality(impact_preview) BETWEEN 1 AND 16),
    completion_status TEXT NOT NULL CHECK (completion_status = 'completed'),
    protected_state_unchanged BOOLEAN NOT NULL CHECK (protected_state_unchanged),
    started_at TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ NOT NULL,
    duration_ms BIGINT NOT NULL CHECK (duration_ms >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (completed_at >= started_at)
);

CREATE INDEX action_plan_simulations_plan_created_idx
    ON action_plan_simulations(plan_id, created_at DESC);

ALTER TABLE action_plans
    ADD CONSTRAINT action_plans_latest_simulation_fk
    FOREIGN KEY (latest_simulation_id) REFERENCES action_plan_simulations(id);

CREATE TABLE action_plan_simulation_audits (
    id UUID PRIMARY KEY,
    request_id UUID NOT NULL,
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    factory_id UUID NOT NULL,
    plan_id UUID NOT NULL,
    actor_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    action TEXT NOT NULL CHECK (action IN ('simulate', 'drift_invalidate')),
    prior_status TEXT NOT NULL CHECK (prior_status = 'validated'),
    resulting_status TEXT NOT NULL CHECK (
        resulting_status IN ('simulated', 'invalidated_by_drift')
    ),
    prior_version BIGINT NOT NULL CHECK (prior_version > 0),
    resulting_version BIGINT NOT NULL CHECK (resulting_version = prior_version + 1),
    simulation_id UUID REFERENCES action_plan_simulations(id),
    source_snapshot_hash TEXT NOT NULL CHECK (source_snapshot_hash ~ '^[0-9a-f]{64}$'),
    observed_snapshot_hash TEXT NOT NULL CHECK (observed_snapshot_hash ~ '^[0-9a-f]{64}$'),
    input_hash TEXT CHECK (input_hash IS NULL OR input_hash ~ '^[0-9a-f]{64}$'),
    output_hash TEXT CHECK (output_hash IS NULL OR output_hash ~ '^[0-9a-f]{64}$'),
    simulation_policy_version TEXT NOT NULL,
    idempotency_outcome TEXT NOT NULL CHECK (
        idempotency_outcome IN ('created', 'replayed')
    ),
    protected_state_unchanged BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (plan_id, tenant_id, factory_id)
        REFERENCES action_plans(id, tenant_id, factory_id),
    FOREIGN KEY (factory_id, tenant_id)
        REFERENCES factories(id, tenant_id)
);

CREATE INDEX action_plan_simulation_audits_plan_created_idx
    ON action_plan_simulation_audits(plan_id, created_at DESC);
