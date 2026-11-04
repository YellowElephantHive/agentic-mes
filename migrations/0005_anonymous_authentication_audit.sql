ALTER TABLE authorization_decision_audits
    ALTER COLUMN tenant_id DROP NOT NULL,
    ALTER COLUMN factory_id DROP NOT NULL;
