CREATE TABLE action_plan_snapshots (
    id UUID PRIMARY KEY,
    plan_id UUID NOT NULL REFERENCES action_plans(id),
    plan_version BIGINT NOT NULL CHECK (plan_version > 0),
    validation_policy_version TEXT NOT NULL CHECK (
        char_length(validation_policy_version) BETWEEN 1 AND 64
    ),
    snapshot_hash TEXT NOT NULL CHECK (snapshot_hash ~ '^[0-9a-f]{64}$'),
    canonical_payload JSONB NOT NULL CHECK (pg_column_size(canonical_payload) <= 65536),
    missing_fields TEXT[] NOT NULL DEFAULT '{}'
        CHECK (cardinality(missing_fields) <= 32),
    target_count INTEGER NOT NULL CHECK (target_count BETWEEN 0 AND 16),
    evidence_count INTEGER NOT NULL CHECK (evidence_count BETWEEN 0 AND 8),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX action_plan_snapshots_plan_created_idx
    ON action_plan_snapshots(plan_id, created_at DESC);

CREATE TABLE action_plan_validation_audits (
    id UUID PRIMARY KEY,
    request_id UUID NOT NULL,
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    factory_id UUID NOT NULL,
    plan_id UUID NOT NULL,
    actor_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    action TEXT NOT NULL CHECK (action IN ('validate', 'drift_invalidate')),
    prior_status TEXT NOT NULL CHECK (prior_status IN ('draft', 'validated')),
    resulting_status TEXT NOT NULL CHECK (
        resulting_status IN ('validated', 'invalidated_by_drift')
    ),
    prior_version BIGINT NOT NULL CHECK (prior_version > 0),
    resulting_version BIGINT NOT NULL CHECK (resulting_version >= prior_version),
    snapshot_id UUID NOT NULL REFERENCES action_plan_snapshots(id),
    snapshot_hash TEXT NOT NULL CHECK (snapshot_hash ~ '^[0-9a-f]{64}$'),
    validation_policy_version TEXT NOT NULL,
    idempotency_outcome TEXT NOT NULL CHECK (
        idempotency_outcome IN ('created', 'replayed')
    ),
    target_count INTEGER NOT NULL CHECK (target_count BETWEEN 0 AND 16),
    evidence_count INTEGER NOT NULL CHECK (evidence_count BETWEEN 0 AND 8),
    missing_field_count INTEGER NOT NULL CHECK (missing_field_count BETWEEN 0 AND 32),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (plan_id, tenant_id, factory_id)
        REFERENCES action_plans(id, tenant_id, factory_id),
    FOREIGN KEY (factory_id, tenant_id)
        REFERENCES factories(id, tenant_id)
);

CREATE INDEX action_plan_validation_audits_plan_created_idx
    ON action_plan_validation_audits(plan_id, created_at DESC);
