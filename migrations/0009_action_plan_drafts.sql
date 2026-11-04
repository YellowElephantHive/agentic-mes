ALTER TABLE disposition_proposals
    ADD CONSTRAINT disposition_proposals_version_scope_idx
    UNIQUE (id, tenant_id, factory_id, version);

ALTER TABLE disposition_reviews
    ADD CONSTRAINT disposition_reviews_source_scope_idx
    UNIQUE (id, proposal_id, tenant_id, factory_id);

CREATE TABLE action_plans (
    id UUID PRIMARY KEY,
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    factory_id UUID NOT NULL,
    created_by_user_id UUID NOT NULL REFERENCES users(id),
    source_proposal_id UUID NOT NULL,
    source_proposal_version BIGINT NOT NULL CHECK (source_proposal_version > 0),
    source_review_id UUID NOT NULL,
    plan_type TEXT NOT NULL CHECK (
        plan_type IN (
            'maintenance_inspection_plan',
            'quality_investigation_plan',
            'inventory_reconciliation_plan'
        )
    ),
    status TEXT NOT NULL CHECK (
        status IN (
            'draft', 'validated', 'simulated', 'pending_approval',
            'plan_approved', 'plan_rejected', 'expired',
            'invalidated_by_drift', 'cancelled'
        )
    ),
    source_snapshot_hash TEXT CHECK (
        source_snapshot_hash IS NULL OR source_snapshot_hash ~ '^[0-9a-f]{64}$'
    ),
    latest_simulation_id UUID,
    summary TEXT NOT NULL CHECK (char_length(summary) BETWEEN 1 AND 2000),
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    validated_at TIMESTAMPTZ,
    submitted_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    UNIQUE (id, tenant_id, factory_id),
    FOREIGN KEY (factory_id, tenant_id)
        REFERENCES factories(id, tenant_id),
    FOREIGN KEY (
        source_proposal_id, tenant_id, factory_id, source_proposal_version
    ) REFERENCES disposition_proposals(id, tenant_id, factory_id, version),
    FOREIGN KEY (
        source_review_id, source_proposal_id, tenant_id, factory_id
    ) REFERENCES disposition_reviews(id, proposal_id, tenant_id, factory_id),
    CHECK (expires_at > created_at)
);

CREATE INDEX action_plans_scope_created_idx
    ON action_plans(tenant_id, factory_id, created_at DESC);
CREATE INDEX action_plans_creator_idx
    ON action_plans(tenant_id, created_by_user_id, created_at DESC);
CREATE INDEX action_plans_source_idx
    ON action_plans(source_proposal_id, created_at DESC);

CREATE TABLE action_plan_targets (
    plan_id UUID NOT NULL REFERENCES action_plans(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    resource_type TEXT NOT NULL CHECK (char_length(resource_type) BETWEEN 1 AND 64),
    resource_id TEXT NOT NULL CHECK (char_length(resource_id) BETWEEN 1 AND 256),
    PRIMARY KEY (plan_id, ordinal),
    UNIQUE (plan_id, resource_type, resource_id)
);

CREATE TABLE action_plan_evidence (
    plan_id UUID NOT NULL REFERENCES action_plans(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    source_proposal_id UUID NOT NULL,
    source_run_id UUID NOT NULL,
    agent_tool_call_id UUID NOT NULL,
    evidence_type TEXT NOT NULL CHECK (char_length(evidence_type) BETWEEN 1 AND 64),
    PRIMARY KEY (plan_id, ordinal),
    UNIQUE (plan_id, agent_tool_call_id),
    FOREIGN KEY (source_proposal_id, source_run_id)
        REFERENCES disposition_proposals(id, source_run_id),
    FOREIGN KEY (agent_tool_call_id, source_run_id)
        REFERENCES agent_tool_calls(id, run_id)
);

CREATE TABLE action_plan_steps (
    plan_id UUID NOT NULL REFERENCES action_plans(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    step_code TEXT NOT NULL CHECK (
        step_code IN (
            'machine_inspection_check', 'downtime_history_review',
            'maintenance_evidence_collection', 'maintenance_role_notification',
            'quality_history_review', 'quality_sample_collection',
            'production_records_review', 'quality_role_notification',
            'material_identity_inspection', 'inventory_records_reconciliation',
            'physical_observation_collection'
        )
    ),
    PRIMARY KEY (plan_id, ordinal),
    UNIQUE (plan_id, step_code)
);
