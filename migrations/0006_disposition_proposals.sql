ALTER TABLE agent_runs
    ADD COLUMN factory_id UUID;

ALTER TABLE agent_runs
    ADD CONSTRAINT agent_runs_factory_scope_fk
    FOREIGN KEY (factory_id, tenant_id)
    REFERENCES factories(id, tenant_id);

CREATE UNIQUE INDEX agent_runs_id_tenant_idx
    ON agent_runs(id, tenant_id);

CREATE TABLE disposition_proposals (
    id UUID PRIMARY KEY,
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    factory_id UUID NOT NULL,
    initiated_by_user_id UUID NOT NULL REFERENCES users(id),
    source_run_id UUID NOT NULL,
    supersedes_proposal_id UUID REFERENCES disposition_proposals(id),
    proposal_type TEXT NOT NULL CHECK (
        proposal_type IN (
            'maintenance_investigation',
            'quality_investigation',
            'material_discrepancy_investigation'
        )
    ),
    status TEXT NOT NULL CHECK (
        status IN (
            'draft', 'pending_review', 'review_approved', 'review_rejected',
            'expired', 'cancelled'
        )
    ),
    evidence_state TEXT NOT NULL CHECK (
        evidence_state IN (
            'supported', 'suspected_mismatch', 'inconclusive',
            'confirmed_match', 'confirmed_mismatch'
        )
    ),
    summary TEXT NOT NULL CHECK (char_length(summary) BETWEEN 1 AND 2000),
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    submitted_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    UNIQUE (id, tenant_id),
    FOREIGN KEY (factory_id, tenant_id)
        REFERENCES factories(id, tenant_id),
    FOREIGN KEY (source_run_id, tenant_id)
        REFERENCES agent_runs(id, tenant_id),
    CHECK (expires_at > created_at)
);

CREATE INDEX disposition_proposals_scope_created_idx
    ON disposition_proposals(tenant_id, factory_id, created_at DESC);
CREATE INDEX disposition_proposals_initiator_idx
    ON disposition_proposals(tenant_id, initiated_by_user_id, created_at DESC);

CREATE TABLE disposition_proposal_targets (
    proposal_id UUID NOT NULL REFERENCES disposition_proposals(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    resource_type TEXT NOT NULL CHECK (char_length(resource_type) BETWEEN 1 AND 64),
    resource_id TEXT NOT NULL CHECK (char_length(resource_id) BETWEEN 1 AND 256),
    PRIMARY KEY (proposal_id, ordinal),
    UNIQUE (proposal_id, resource_type, resource_id)
);

CREATE TABLE disposition_proposal_evidence (
    proposal_id UUID NOT NULL REFERENCES disposition_proposals(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    evidence_type TEXT NOT NULL CHECK (char_length(evidence_type) BETWEEN 1 AND 64),
    evidence_id TEXT NOT NULL CHECK (char_length(evidence_id) BETWEEN 1 AND 256),
    PRIMARY KEY (proposal_id, ordinal),
    UNIQUE (proposal_id, evidence_type, evidence_id)
);

CREATE TABLE disposition_proposal_actions (
    proposal_id UUID NOT NULL REFERENCES disposition_proposals(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    recommendation_code TEXT NOT NULL CHECK (
        recommendation_code IN (
            'inspect_machine', 'request_maintenance_review',
            'request_quality_review', 'review_production_records',
            'inspect_material_identity', 'reconcile_inventory'
        )
    ),
    PRIMARY KEY (proposal_id, ordinal),
    UNIQUE (proposal_id, recommendation_code)
);

CREATE TABLE disposition_reviews (
    id UUID PRIMARY KEY,
    proposal_id UUID NOT NULL REFERENCES disposition_proposals(id),
    proposal_version BIGINT NOT NULL CHECK (proposal_version > 0),
    reviewer_user_id UUID NOT NULL REFERENCES users(id),
    decision TEXT NOT NULL CHECK (decision IN ('approved', 'rejected')),
    rationale TEXT CHECK (rationale IS NULL OR char_length(rationale) <= 2000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE idempotency_records (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    actor_user_id UUID NOT NULL REFERENCES users(id),
    operation TEXT NOT NULL CHECK (char_length(operation) BETWEEN 1 AND 128),
    idempotency_key TEXT NOT NULL CHECK (char_length(idempotency_key) BETWEEN 1 AND 128),
    request_hash TEXT NOT NULL CHECK (request_hash ~ '^[0-9a-f]{64}$'),
    resource_type TEXT NOT NULL CHECK (char_length(resource_type) BETWEEN 1 AND 64),
    resource_id UUID NOT NULL,
    response_status INTEGER NOT NULL CHECK (response_status BETWEEN 100 AND 599),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, actor_user_id, operation, idempotency_key)
);
