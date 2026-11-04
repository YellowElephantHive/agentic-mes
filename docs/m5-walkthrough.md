# M5 reproducible walkthrough

Prerequisites: Rust 1.85+, Docker with Compose, `curl`, and outbound access to the pinned UCI source archives on the first run.

From a clean checkout, run:

```bash
./scripts/reproduce-mvp.sh
```

The script performs these independently checked stages:

1. starts PostgreSQL and waits for its health check;
2. applies migrations and repeats the deterministic `seed=42` factory seed;
3. starts the API when necessary and executes a tenant/factory-scoped MES query with `curl`;
4. asks the deterministic mock Agent why Line A output dropped, then reads the persisted run and four tool-call audits;
5. downloads checksum-pinned Track A and Track B source archives, preserving the published records and missing values;
6. imports each dataset through its own adapter and regenerates the two independent reports.

The command exits nonzero at the first failed stage. Its final three lines must be:

```text
M5 walkthrough: PASS
Track A: PASS
Track B: PASS
```

The deterministic mock is intentional for reproducible CI. M4 separately verifies the real-provider protocol. Neither the walkthrough nor a model is allowed to write MES data or execute SQL.

## Manual audit inspection

The walkthrough prints the run UUID and audit result. It stops containers but preserves the Docker volume. Restart PostgreSQL with `docker compose up --detach --wait db`, then run the equivalent operator-only SQL if deeper inspection is needed:

```sql
SELECT id, tenant_id, user_id, agent_id, status, error_code, started_at, finished_at
FROM agent_runs
WHERE id = '<run-id>';

SELECT tool_name, auth_decision, duration_ms, created_at
FROM agent_tool_calls
WHERE run_id = '<run-id>'
ORDER BY created_at;
```

Do not inspect runs without a tenant predicate in application code. The SQL above is an operator-only local debugging example.

Application clients should use the tenant-scoped endpoints instead:

```http
GET /api/v1/agent/runs/{run_id}
GET /api/v1/agent/runs/{run_id}/tool-calls
```
