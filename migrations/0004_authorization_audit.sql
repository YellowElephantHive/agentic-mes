CREATE TABLE authorization_decision_audits (
    id UUID PRIMARY KEY,
    request_id UUID NOT NULL,
    user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    external_identity_id UUID REFERENCES external_identities(id) ON DELETE SET NULL,
    tenant_id UUID NOT NULL,
    factory_id UUID NOT NULL,
    auth_method TEXT NOT NULL CHECK (auth_method IN ('dev_headers', 'oidc')),
    issuer TEXT NOT NULL,
    roles TEXT[] NOT NULL,
    capabilities TEXT[] NOT NULL,
    action TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    resource_id TEXT,
    outcome TEXT NOT NULL CHECK (outcome IN ('authorized', 'denied')),
    denial_category TEXT,
    duration_ms BIGINT NOT NULL CHECK (duration_ms >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX authorization_decision_request_idx
    ON authorization_decision_audits(request_id);
CREATE INDEX authorization_decision_actor_idx
    ON authorization_decision_audits(tenant_id, user_id, created_at DESC);
