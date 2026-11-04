# Agentic MES

Rust middleware for a read-only, auditable Agent-as-a-Service layer over existing Manufacturing Execution Systems (MES). This repository is being implemented one reviewed milestone at a time. It is an executable MVP project, not a production-certified MES.

## Current status

Milestones **M0 — Scaffold** through **M6 — Real-provider Agent Service** are implemented and independently verified. The executable MVP status is **PASS**; this is not a production-security or production-MES certification.

The product boundary is strict:

- MES business logic must work without an LLM.
- Agents never receive raw SQL, database credentials, PLC access, or uncontrolled write access.
- Missing manufacturing values remain missing; they are never fabricated or silently imputed.
- Original source records and provenance must be retained by later dataset adapters.
- The MVP cannot be declared complete until both mandatory evaluation tracks pass independently.

See [Dual-track evaluation](docs/dual-track-evaluation.md) for the binding evaluation gates.

Current evaluation status:

- Track A — UCI AI4I 2020: **PASS**
- Track B — UCI SECOM: **PASS**

These passes cover the frozen source-dataset agent-tool protocol. Because M0–M6 and both mandatory tracks pass, the executable MVP acceptance gate is complete.

## Workspace

| Crate | Responsibility |
|---|---|
| `mes-domain` | Entities, value types, and invariants |
| `mes-application` | Use cases, repository traits, and metric calculations |
| `mes-infra` | PostgreSQL/SQLx repository implementations |
| `agent-core` | Tool registry, policy, and bounded execution loop |
| `llm-provider` | Provider trait, deterministic mock, generic HTTP and Ollama adapters |
| `factory-simulator` | Reproducible synthetic factory seeding |
| `mes-api` | Axum HTTP API and dependency wiring |

Dependencies point inward: application depends on domain; infrastructure depends on application and domain. Domain has no agent, provider, infrastructure, or API dependencies.

## Local checks

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Run the API directly:

```bash
APP_BIND_ADDR=127.0.0.1:8080 cargo run --package mes-api
curl --fail http://127.0.0.1:8080/healthz
curl --fail http://127.0.0.1:8080/readyz
```

Expected response:

```json
{"status":"ok","service":"mes-api"}
```

The authenticated routes use `x-dev-tenant-id`, `x-dev-factory-id`, and optionally `x-dev-user-id` only while `APP_ENV=development`. The API also exposes the bounded Agent lifecycle:

```http
POST /api/v1/agent/runs
GET  /api/v1/agent/runs/{run_id}
GET  /api/v1/agent/runs/{run_id}/tool-calls
```

Request tracing, generated `x-request-id` headers, a 64 KiB body limit, and a configurable per-user/tenant in-memory rate limit are enabled. Production authentication remains intentionally unimplemented, so non-development startup fails closed.

M6 selects the serving provider at startup with `AGENT_PROVIDER=mock|ollama|http`. Ollama is local-only; the generic external HTTP provider fails startup unless `LLM_DATA_POLICY=external_approved`. Runtime timeouts, provider concurrency and the circuit breaker are bounded by `AGENT_*` settings. Agent evidence is rebuilt from executed tool results, and the Line A endpoint fails closed unless both production windows plus current downtime and quality calls completed.

Run the API directly with the verified local Ollama configuration:

```bash
AGENT_PROVIDER=ollama \
LLM_DATA_POLICY=local_only \
OLLAMA_MODEL=gemma4:e4b \
AGENT_PROVIDER_TIMEOUT_MS=60000 \
AGENT_OVERALL_TIMEOUT_MS=180000 \
DATABASE_URL=postgres://agentic_mes:agentic_mes@127.0.0.1:5432/agentic_mes \
cargo run --package mes-api
```

Run the M0 stack:

```bash
cp .env.example .env
docker compose up --build --wait
curl --fail http://127.0.0.1:8080/healthz
docker compose down
```

The database is started and health-checked in M0. M1 migrations and the deterministic seed run through the simulator:

```bash
DATABASE_URL=postgres://agentic_mes:agentic_mes@127.0.0.1:5432/agentic_mes \
  cargo run --package factory-simulator -- \
  seed --seed 42 --days 30 --factory demo
```

M1's fixed fixture window is 2026-01-01 through 2026-01-30 UTC. It is intentionally independent of the current date.

## Evaluation datasets

Dataset bytes are not committed. Fetch the pinned M1 source archives:

```bash
./scripts/fetch-evaluation-datasets.sh
```

Before publishing or cloning the project, see [GitHub publishing and clean-clone guide](docs/github-publishing.md) for the committed-file policy, automatic dataset acquisition, secret handling and pre-push checklist.

Preview or persist the Track A adapter:

```bash
cargo run --package factory-simulator -- \
  import-ai4i --csv data/evaluation/ai4i/ai4i2020.csv
```

Preview or persist the Track B adapter:

```bash
cargo run --package factory-simulator -- \
  import-secom \
  --data data/evaluation/secom/secom.data \
  --labels data/evaluation/secom/secom_labels.data
```

Without `DATABASE_URL`, imports only validate and print their manifest. With it, immutable raw and normalized records are persisted. See [M1 report](docs/m1-report.md) and [dataset provenance](docs/dataset-provenance.md).

## Milestone sequence

1. M0: workspace, health endpoint, Docker Compose, CI and documentation.
2. M1: data contracts, migrations, source-preserving adapters, simulator and fixtures. **Implemented.**
3. M2: tenant-scoped MES read APIs. **Implemented.**
4. M3: source-grounded agent tools and deterministic mock-provider runtime. **Implemented.**
5. M4: real provider adapter without policy bypass. **Implemented.**
6. M5: hardening, reproducible Track A and Track B suites, and separate reports. **Implemented.**
7. M6: selectable real-provider serving, data policy, grounded-evidence gate and provider resilience. **Implemented.**

Passing one evaluation track does not compensate for failing or unevaluated performance on the other.

Run the complete clean-checkout acceptance flow with:

```bash
./scripts/reproduce-mvp.sh
```

See [M5 walkthrough](docs/m5-walkthrough.md), [M5 report](docs/m5-report.md), and [M6 report](docs/m6-report.md).
