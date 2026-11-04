CREATE TABLE action_plan_reviews (
    id UUID PRIMARY KEY,
    plan_id UUID NOT NULL REFERENCES action_plans(id),
    plan_version BIGINT NOT NULL CHECK (plan_version > 0),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    factory_id UUID NOT NULL,
    reviewer_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    decision TEXT NOT NULL CHECK (decision IN ('approved', 'rejected')),
    rationale TEXT CHECK (rationale IS NULL OR char_length(rationale) <= 2000),
    source_snapshot_hash TEXT NOT NULL CHECK (source_snapshot_hash ~ '^[0-9a-f]{64}$'),
    simulation_id UUID NOT NULL REFERENCES action_plan_simulations(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (plan_id),
    FOREIGN KEY (plan_id, tenant_id, factory_id)
        REFERENCES action_plans(id, tenant_id, factory_id),
    FOREIGN KEY (factory_id, tenant_id)
        REFERENCES factories(id, tenant_id)
);

CREATE TABLE action_plan_transition_audits (
    id UUID PRIMARY KEY,
    request_id UUID NOT NULL,
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    factory_id UUID NOT NULL,
    plan_id UUID NOT NULL,
    actor_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    action TEXT NOT NULL CHECK (
        action IN ('submit', 'cancel', 'expire', 'review', 'drift_invalidate')
    ),
    prior_status TEXT NOT NULL,
    resulting_status TEXT NOT NULL,
    prior_version BIGINT NOT NULL CHECK (prior_version > 0),
    resulting_version BIGINT NOT NULL CHECK (resulting_version = prior_version + 1),
    snapshot_hash TEXT CHECK (snapshot_hash IS NULL OR snapshot_hash ~ '^[0-9a-f]{64}$'),
    simulation_id UUID REFERENCES action_plan_simulations(id),
    review_id UUID REFERENCES action_plan_reviews(id),
    decision TEXT CHECK (decision IS NULL OR decision IN ('approved', 'rejected')),
    idempotency_outcome TEXT NOT NULL CHECK (
        idempotency_outcome IN ('created', 'not_applicable')
    ),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (plan_id, tenant_id, factory_id)
        REFERENCES action_plans(id, tenant_id, factory_id),
    FOREIGN KEY (factory_id, tenant_id)
        REFERENCES factories(id, tenant_id)
);

CREATE INDEX action_plan_transition_audits_plan_created_idx
    ON action_plan_transition_audits(plan_id, created_at DESC);
