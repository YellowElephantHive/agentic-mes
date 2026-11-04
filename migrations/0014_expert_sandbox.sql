CREATE TABLE domain_feedback (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    factory_id UUID NOT NULL,
    actor_user_id UUID NOT NULL REFERENCES users(id),
    run_id UUID NOT NULL,
    proposal_id UUID,
    plan_id UUID,
    evidence_rating TEXT NOT NULL CHECK (
        evidence_rating IN ('reasonable', 'missing_key_signal', 'over_inference')
    ),
    realism_rating TEXT NOT NULL CHECK (
        realism_rating IN ('realistic', 'too_clean', 'contradictory')
    ),
    next_source TEXT CHECK (next_source IS NULL OR char_length(next_source) <= 2000),
    missing_factor TEXT CHECK (missing_factor IS NULL OR char_length(missing_factor) <= 2000),
    consent_for_research BOOLEAN NOT NULL DEFAULT false,
    trace_metadata JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (factory_id, tenant_id) REFERENCES factories(id, tenant_id),
    FOREIGN KEY (run_id, tenant_id) REFERENCES agent_runs(id, tenant_id),
    FOREIGN KEY (proposal_id, tenant_id) REFERENCES disposition_proposals(id, tenant_id),
    FOREIGN KEY (plan_id, tenant_id, factory_id) REFERENCES action_plans(id, tenant_id, factory_id)
);

CREATE INDEX domain_feedback_scope_created_idx
    ON domain_feedback(tenant_id, factory_id, created_at DESC);
CREATE INDEX domain_feedback_run_idx ON domain_feedback(run_id, created_at DESC);
