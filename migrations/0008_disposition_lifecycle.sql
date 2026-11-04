ALTER TABLE disposition_proposals
    ADD CONSTRAINT disposition_proposals_id_scope_idx
    UNIQUE (id, tenant_id, factory_id);

ALTER TABLE disposition_reviews
    ADD COLUMN tenant_id UUID NOT NULL,
    ADD COLUMN factory_id UUID NOT NULL,
    ADD CONSTRAINT disposition_reviews_proposal_scope_fk
        FOREIGN KEY (proposal_id, tenant_id, factory_id)
        REFERENCES disposition_proposals(id, tenant_id, factory_id),
    ADD CONSTRAINT disposition_reviews_one_decision UNIQUE (proposal_id);

CREATE TABLE disposition_transition_audits (
    id UUID PRIMARY KEY,
    request_id UUID NOT NULL,
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    factory_id UUID NOT NULL,
    proposal_id UUID NOT NULL,
    source_run_id UUID NOT NULL,
    actor_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    action TEXT NOT NULL CHECK (action IN ('submit', 'review', 'cancel', 'expire')),
    prior_status TEXT NOT NULL CHECK (
        prior_status IN (
            'draft', 'pending_review', 'review_approved', 'review_rejected',
            'expired', 'cancelled'
        )
    ),
    resulting_status TEXT NOT NULL CHECK (
        resulting_status IN (
            'draft', 'pending_review', 'review_approved', 'review_rejected',
            'expired', 'cancelled'
        )
    ),
    prior_version BIGINT NOT NULL CHECK (prior_version > 0),
    resulting_version BIGINT NOT NULL CHECK (resulting_version > prior_version),
    outcome TEXT NOT NULL CHECK (outcome IN ('completed', 'denied')),
    denial_category TEXT,
    idempotency_outcome TEXT NOT NULL CHECK (
        idempotency_outcome IN ('created', 'replayed', 'not_applicable')
    ),
    evidence_reference_count INTEGER NOT NULL CHECK (evidence_reference_count >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (proposal_id, tenant_id, factory_id)
        REFERENCES disposition_proposals(id, tenant_id, factory_id),
    FOREIGN KEY (factory_id, tenant_id)
        REFERENCES factories(id, tenant_id)
);

CREATE INDEX disposition_transition_audits_proposal_idx
    ON disposition_transition_audits(proposal_id, created_at);
