CREATE TABLE users (
    id UUID PRIMARY KEY,
    display_name TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'suspended')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE external_identities (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    issuer TEXT NOT NULL,
    subject TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (issuer, subject)
);

CREATE TABLE tenant_memberships (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('active', 'suspended')),
    all_factories BOOLEAN NOT NULL DEFAULT false,
    UNIQUE (user_id, tenant_id),
    UNIQUE (id, tenant_id)
);

CREATE TABLE tenant_membership_roles (
    membership_id UUID NOT NULL REFERENCES tenant_memberships(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('viewer', 'production_analyst', 'supervisor', 'admin')),
    PRIMARY KEY (membership_id, role)
);

CREATE TABLE membership_factory_scopes (
    membership_id UUID NOT NULL,
    tenant_id UUID NOT NULL,
    factory_id UUID NOT NULL,
    PRIMARY KEY (membership_id, factory_id),
    FOREIGN KEY (membership_id, tenant_id)
        REFERENCES tenant_memberships(id, tenant_id) ON DELETE CASCADE,
    FOREIGN KEY (factory_id, tenant_id)
        REFERENCES factories(id, tenant_id) ON DELETE CASCADE
);

CREATE INDEX external_identities_user_idx ON external_identities(user_id);
CREATE INDEX tenant_memberships_tenant_idx ON tenant_memberships(tenant_id, user_id);
