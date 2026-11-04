CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE tenants (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE factories (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    slug TEXT NOT NULL,
    timezone TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (id, tenant_id),
    UNIQUE (tenant_id, slug)
);

CREATE TABLE production_lines (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    factory_id UUID NOT NULL,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (id, factory_id, tenant_id),
    UNIQUE (factory_id, name),
    FOREIGN KEY (factory_id, tenant_id)
        REFERENCES factories(id, tenant_id) ON DELETE CASCADE
);

CREATE TABLE machines (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    factory_id UUID NOT NULL,
    line_id UUID NOT NULL,
    name TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('available', 'down', 'maintenance')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (id, line_id, factory_id, tenant_id),
    UNIQUE (line_id, name),
    FOREIGN KEY (line_id, factory_id, tenant_id)
        REFERENCES production_lines(id, factory_id, tenant_id) ON DELETE CASCADE
);

CREATE TABLE products (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    sku TEXT NOT NULL,
    ideal_cycle_seconds DOUBLE PRECISION NOT NULL CHECK (ideal_cycle_seconds > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (id, tenant_id),
    UNIQUE (tenant_id, sku)
);

CREATE TABLE work_orders (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    factory_id UUID NOT NULL,
    line_id UUID NOT NULL,
    product_id UUID NOT NULL,
    status TEXT NOT NULL CHECK (
        status IN ('planned', 'released', 'in_progress', 'completed', 'cancelled')
    ),
    planned_qty BIGINT NOT NULL CHECK (planned_qty > 0),
    due_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (id, line_id, factory_id, tenant_id),
    FOREIGN KEY (line_id, factory_id, tenant_id)
        REFERENCES production_lines(id, factory_id, tenant_id) ON DELETE CASCADE,
    FOREIGN KEY (product_id, tenant_id)
        REFERENCES products(id, tenant_id) ON DELETE CASCADE
);

CREATE OR REPLACE FUNCTION enforce_work_order_transition()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.status = OLD.status OR
       (OLD.status = 'planned' AND NEW.status IN ('released', 'cancelled')) OR
       (OLD.status = 'released' AND NEW.status IN ('in_progress', 'cancelled')) OR
       (OLD.status = 'in_progress' AND NEW.status IN ('completed', 'cancelled')) THEN
        NEW.updated_at = now();
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'invalid work order transition from % to %', OLD.status, NEW.status;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER work_order_transition_guard
BEFORE UPDATE OF status ON work_orders
FOR EACH ROW EXECUTE FUNCTION enforce_work_order_transition();

CREATE TABLE production_records (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    factory_id UUID NOT NULL,
    line_id UUID NOT NULL,
    work_order_id UUID NOT NULL,
    machine_id UUID NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    good_qty BIGINT NOT NULL CHECK (good_qty >= 0),
    reject_qty BIGINT NOT NULL CHECK (reject_qty >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (id, tenant_id),
    FOREIGN KEY (work_order_id, line_id, factory_id, tenant_id)
        REFERENCES work_orders(id, line_id, factory_id, tenant_id) ON DELETE CASCADE,
    FOREIGN KEY (machine_id, line_id, factory_id, tenant_id)
        REFERENCES machines(id, line_id, factory_id, tenant_id) ON DELETE CASCADE
);

CREATE INDEX production_records_scope_time_idx
    ON production_records (tenant_id, factory_id, line_id, occurred_at);

CREATE TABLE downtime_events (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    factory_id UUID NOT NULL,
    line_id UUID NOT NULL,
    machine_id UUID NOT NULL,
    started_at TIMESTAMPTZ NOT NULL,
    ended_at TIMESTAMPTZ NOT NULL,
    reason_code TEXT NOT NULL,
    source_scenario_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (ended_at >= started_at),
    UNIQUE (id, tenant_id),
    FOREIGN KEY (machine_id, line_id, factory_id, tenant_id)
        REFERENCES machines(id, line_id, factory_id, tenant_id) ON DELETE CASCADE
);

CREATE INDEX downtime_events_scope_time_idx
    ON downtime_events (tenant_id, factory_id, line_id, started_at, ended_at);

CREATE TABLE quality_inspections (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    factory_id UUID NOT NULL,
    line_id UUID NOT NULL,
    work_order_id UUID NOT NULL,
    inspected_at TIMESTAMPTZ NOT NULL,
    sample_size BIGINT NOT NULL CHECK (sample_size >= 0),
    defect_count BIGINT NOT NULL CHECK (defect_count >= 0),
    defect_code TEXT,
    source_scenario_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (defect_count <= sample_size),
    UNIQUE (id, tenant_id),
    FOREIGN KEY (work_order_id, line_id, factory_id, tenant_id)
        REFERENCES work_orders(id, line_id, factory_id, tenant_id) ON DELETE CASCADE
);

CREATE INDEX quality_inspections_scope_time_idx
    ON quality_inspections (tenant_id, factory_id, line_id, inspected_at);

CREATE TABLE dataset_imports (
    id UUID PRIMARY KEY,
    tenant_id UUID REFERENCES tenants(id) ON DELETE CASCADE,
    dataset_key TEXT NOT NULL,
    evaluation_track TEXT NOT NULL CHECK (
        evaluation_track IN ('published_synthetic', 'real_world')
    ),
    dataset_name TEXT NOT NULL,
    dataset_version TEXT NOT NULL,
    license TEXT NOT NULL,
    source_url TEXT NOT NULL,
    source_files TEXT[] NOT NULL,
    sha256 TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    imported_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    record_count BIGINT NOT NULL CHECK (record_count >= 0),
    UNIQUE (dataset_key, dataset_version, sha256)
);

CREATE TABLE dataset_source_records (
    id UUID PRIMARY KEY,
    import_id UUID NOT NULL REFERENCES dataset_imports(id) ON DELETE CASCADE,
    source_record_id TEXT NOT NULL,
    source_locator TEXT NOT NULL,
    raw_payload JSONB NOT NULL,
    normalized_payload JSONB NOT NULL,
    missing_fields TEXT[] NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (import_id, source_record_id)
);

CREATE INDEX dataset_source_records_import_idx
    ON dataset_source_records (import_id, source_record_id);

CREATE TABLE agent_runs (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (
        status IN ('queued', 'running', 'completed', 'failed', 'timed_out', 'cancelled')
    ),
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ,
    error_code TEXT,
    final_response TEXT
);

CREATE TABLE agent_tool_calls (
    id UUID PRIMARY KEY,
    run_id UUID NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    tool_name TEXT NOT NULL,
    redacted_arguments JSONB NOT NULL,
    auth_decision TEXT NOT NULL,
    result_summary JSONB,
    duration_ms BIGINT NOT NULL CHECK (duration_ms >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE audit_events (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    actor_id TEXT NOT NULL,
    action TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    outcome TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX audit_events_tenant_created_idx
    ON audit_events (tenant_id, created_at);
