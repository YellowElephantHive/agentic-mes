# Agentic MES

Rust middleware for a read-only, auditable Agent-as-a-Service layer over existing Manufacturing Execution Systems (MES). This repository is being implemented one reviewed milestone at a time. It is an executable MVP project, not a production-certified MES.

> **Open to work:** I am seeking Backend, AI Infrastructure, and Manufacturing
> Software Engineering opportunities, with particular interest in Rust, agent
> systems, MES, and industrial data platforms. [Contact me through my GitHub profile](https://github.com/YellowElephantHive).

## Current status

Milestones **M0 — Scaffold** through **M6 — Real-provider Agent Service** are implemented and independently verified. **M6.1 — Continuous Dual-Track Evaluation** is passing on GitHub Actions. **M7.0 — Authentication Contract** through **M7.3 — Security Verification** are implemented and passing. **M8.0 — Evidence-bound Disposition Contract** through **M8.2 — Server-rebuilt Proposal Generation** are implemented and passing. **M8.3 — Human Review Lifecycle** is implemented and locally passing; GitHub verification is recorded separately. The executable MVP and completed milestone gates remain **PASS**; this is not a production-security or production-MES certification.

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
- M6.1 GitHub dual-track gate: **PASS**

These passes cover the frozen source-dataset agent-tool protocol. Because M0–M6 and both mandatory tracks pass, the executable MVP acceptance gate is complete.

## MES records are not automatically physical truth

A valid MES transaction does not prove that the recorded physical action
occurred. For example, an operator may report material A while physically
loading material B.

A future read-only investigation layer could compare declared material
records, inventory movements, equipment telemetry, quality results, and
approved substitutions—but it must not infer the actual material from process
anomalies alone. Evidence for such a material-truth capability must be
classified as:

- `CONFIRMED_MISMATCH`: independent physical evidence, such as trusted RFID or
  weigh-scale records, contradicts the MES record.
- `SUSPECTED_MISMATCH`: inventory, process, or quality evidence is inconsistent
  with the declared material.
- `INCONCLUSIVE`: evidence is missing or cannot distinguish the materials.
- `CONFIRMED_MATCH`: independent physical evidence supports the MES record.

Prevention requires deterministic controls such as RFID, weigh-scale
interlocks, controlled material genealogy, and human-approved substitution
workflows. The Agent can reduce investigation cost and preserve the evidence
chain; it cannot replace those controls or accuse an operator based only on
statistical similarity.

The current AI4I and SECOM evaluation datasets do not contain ground-truth
declared-versus-actual material identities. Therefore this repository does not
claim that material-substitution detection is implemented or validated. This
boundary is retained as a restricted reference case in the
[M8 disposition contract](docs/m8-disposition-contract.md). M8.1 implements
the bounded proposal schema and draft API, but not material-substitution
detection or validation.

## Concrete example: audited Line A diagnosis

With the Docker stack running and the deterministic seed command shown under
[Local checks](#local-checks), seed `42` exposes the same demo tenant, factory,
and Line A scope. A caller asks the read-only production analyst to explain the
final-day output drop:

```bash
curl --request POST http://127.0.0.1:8080/api/v1/agent/runs \
  --header 'content-type: application/json' \
  --header 'x-dev-tenant-id: b242333d-c1c1-581c-a71f-ab8b8a92dc2a' \
  --header 'x-dev-factory-id: 8642cb8c-d730-5ce2-a6b0-17f17c2f2444' \
  --data '{"agent_id":"production-analyst","factory_id":"8642cb8c-d730-5ce2-a6b0-17f17c2f2444","line_id":"90c31f01-e9c2-56d9-b365-9d0047c7939c","start_at":"2026-01-30T00:00:00Z","end_at":"2026-01-31T00:00:00Z","message":"Why did Line A output drop?"}'
```

Representative abridged deterministic-mock response (`run_id` and evidence
record UUIDs are generated or resolved at runtime):

```json
{
  "data": {
    "run_id": "<runtime-uuid>",
    "status": "completed",
    "answer": "Line A good output dropped from 1000 to 600 units. Downtime records list MACHINE_FAILURE and MATERIAL_SHORTAGE. The QC sample defect rate was 2.0%. These are source-grounded contributing hypotheses, not proof of causation.",
    "evidence": [
      "production_record:<uuid>",
      "downtime_event:<uuid>",
      "quality_inspection:<uuid>"
    ],
    "tool_calls": 4
  }
}
```

The four required calls are two production windows plus current downtime and
quality summaries. The server rebuilds `evidence` from executed tool results;
it does not accept the provider's claimed citations. If any required call is
missing, the run is persisted as failed and the API returns
`incomplete_tool_evidence` instead of an ungrounded answer. Inspect the durable
trail with `GET /api/v1/agent/runs/{run_id}` and
`GET /api/v1/agent/runs/{run_id}/tool-calls`.

## Architecture and trust flow

```mermaid
flowchart LR
    C[Untrusted client] --> A[dev_headers or OIDC authentication]
    A --> P[Locally authorized principal]
    P --> API[Axum read-only API]
    API --> R[Bounded Agent runtime]
    R -->|prompt and observations| L[LLM provider]
    L -->|tool request or final answer| R
    R -->|authorized tool call| T[Typed tool registry]
    P -->|server-derived scope| T
    T --> Q[Tenant/factory-scoped repositories]
    Q --> DB[(PostgreSQL)]
    Q -->|tool result| R
    R -->|final answer and executed observations| G[Mandatory tool gate]
    G --> E[Deterministic evidence rebuild]
    E --> O[Audited response]
    G -->|incomplete evidence| F[Fail closed]
```

The LLM has no raw SQL, database credentials, PLC access, or authority to add
tenant/factory scope. PostgreSQL repository filters remain mandatory below the
HTTP and Agent boundaries.

## M8.2 evidence-bound disposition drafts

An authorized analyst can create an internal investigation draft from a
completed Agent run:

```http
POST /api/v1/agent/runs/{run_id}/disposition-proposals
Idempotency-Key: case-2026-001
Content-Type: application/json

{
  "proposal_type": "maintenance_investigation"
}
```

The caller can choose only a frozen proposal type. The server rebuilds
targets and evidence references from authorized, successfully executed tool
results and generates bounded summary text and allowlisted recommendations.
Matching production plus downtime or quality profiles produce `supported`;
missing or mismatched evidence produces `inconclusive` or fails closed.
Material-discrepancy proposals remain `inconclusive` until a reviewed physical
evidence adapter exists. Reusing the key with the same canonical request
returns the original draft; reusing it with a different request returns `409
proposal_conflict`. Authorized callers can read the draft through
`GET /api/v1/disposition-proposals/{proposal_id}`.

M8.3 adds human-controlled lifecycle endpoints:

```http
POST /api/v1/disposition-proposals/{proposal_id}/submit
POST /api/v1/disposition-proposals/{proposal_id}/cancel
POST /api/v1/disposition-proposals/{proposal_id}/reviews
Idempotency-Key: transition-001
If-Match: 1
```

Only the initiator can submit or cancel. A different locally authorized user
with `review:disposition_proposals` can approve or reject a pending proposal.
Every transition increments the optimistic version and records a sanitized
transition audit. Due pending proposals become `expired` using the server
clock, after resource authorization. Terminal states cannot transition.

This remains an internal proposal record, not a production action. Approval
records human acceptance of a recommendation only; it does not write to MES,
ERP, PLC, equipment, inventory, quality, material, or work-order state. There
is deliberately no execution route or execution capability.

## Implemented and intentionally not implemented

| Implemented and verified | Not implemented / not claimed |
|---|---|
| Tenant/factory-scoped MES read APIs | Direct MES writes or autonomous production control |
| Typed, bounded read-only Agent tools | Raw text-to-SQL or unrestricted tool execution |
| Tool-result evidence reconstruction and mandatory-call gate | Proof that an Agent hypothesis is the physical root cause |
| Original dataset records, provenance, and explicit missingness | Silent missing-value imputation or fabricated manufacturing data |
| Mock, local Ollama, and policy-gated external HTTP providers | General support for every model vendor or protocol |
| OIDC/JWKS verification and local identity/membership persistence | Identity-provider administration or login UI |
| Endpoint/tool capabilities and self/tenant Agent-run visibility | Unrestricted cross-tenant or cross-factory visibility |
| Sanitized authorization audits, adversarial JWT/JWKS tests, and key rotation | Real production-IdP interoperability or penetration testing |
| Frozen M8 evidence/proposal/review contract | MES write execution or autonomous approval |
| M8.1 typed persistence, M8.2 evidence generation, and M8.3 independent human-review lifecycle | Proposal execution or any operational MES/ERP/PLC write |
| Independent Track A and Track B GitHub evaluation jobs | Production identity-provider validation |
| Reproducible simulator, Docker workflow, tests, and audit records | Production certification, HA/DR, real-factory validation, or a hosted SaaS |

The project is an engineering prototype and executable portfolio artifact. It
must not be represented as a deployed or certified production MES.

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
APP_ENV=development AUTH_PROVIDER=dev_headers APP_BIND_ADDR=127.0.0.1:8080 cargo run --package mes-api
curl --fail http://127.0.0.1:8080/healthz
curl --fail http://127.0.0.1:8080/readyz
```

Expected response:

```json
{"status":"ok","service":"mes-api"}
```

The authenticated routes use `x-dev-tenant-id`, `x-dev-factory-id`, and optionally `x-dev-user-id` only with the explicit `APP_ENV=development AUTH_PROVIDER=dev_headers` configuration. The simulator seeds the only accepted development identity (`dev-user`) and its tenant/factory membership; arbitrary header identities are denied. The API also exposes the bounded Agent lifecycle:

```http
POST /api/v1/agent/runs
GET  /api/v1/agent/runs/{run_id}
GET  /api/v1/agent/runs/{run_id}/tool-calls
```

Request tracing, generated `x-request-id` headers, a 64 KiB body limit, and a configurable per-user/tenant in-memory rate limit are enabled. M7.1 adds signed RS256 OIDC access-token verification, bounded JWKS caching, and database-authoritative identity/membership resolution. M7.2 enforces endpoint, Agent-tool, and self/tenant Agent-run capabilities. M7.3 adds durable sanitized authorization decisions, adversarial JWT/JWKS coverage, and key-rotation verification. Non-development startup requires `AUTH_PROVIDER=oidc`. See the [M7 authentication contract](docs/m7-auth-contract.md) and [formal M7 report](docs/m7-report.md).

M6 selects the serving provider at startup with `AGENT_PROVIDER=mock|ollama|http`. Ollama is local-only; the generic external HTTP provider fails startup unless `LLM_DATA_POLICY=external_approved`. Runtime timeouts, provider concurrency and the circuit breaker are bounded by `AGENT_*` settings. Agent evidence is rebuilt from executed tool results, and the Line A endpoint fails closed unless both production windows plus current downtime and quality calls completed.

Run the API directly with the verified local Ollama configuration:

```bash
AGENT_PROVIDER=ollama \
APP_ENV=development \
AUTH_PROVIDER=dev_headers \
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

Run either evaluation track independently against a local PostgreSQL service:

```bash
./scripts/evaluate-track.sh a
./scripts/evaluate-track.sh b
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
8. M6.1: independent GitHub Actions evaluation jobs, per-track artifacts and an aggregate dual-track gate. **Implemented.**
9. M7.0: production identity, JWT/OIDC, local membership, RBAC and audit contract. **Frozen and passing.**
10. M7.1: identity persistence, explicit auth-provider selection, OIDC/JWKS verification, local membership resolution and principal construction. **Implemented and passing.**
11. M7.2: endpoint and Agent-tool capability enforcement. **Implemented and passing.**
12. M7.3: extended audit, adversarial auth suite, key rotation and formal M7 report. **Implemented and passing.**
13. M8.0: evidence-bound disposition proposal, independent review, and no-execution contract. **Frozen and passing.**
14. M8.1: typed proposal persistence, allowlisted draft creation, idempotency, and authorized read API. **Implemented and passing.**
15. M8.2: server-rebuilt evidence/targets, per-type profiles, bounded generation, and fail-closed validation. **Implemented and passing.**
16. M8.3: submission, independent approval/rejection, cancellation, expiry, optimistic versions, idempotency, and transition audit. **Implemented and locally passing.**
17. M8.4: complete adversarial suite and formal M8 evaluation. **Pending.**

Passing one evaluation track does not compensate for failing or unevaluated performance on the other.

Run the complete clean-checkout acceptance flow with:

```bash
./scripts/reproduce-mvp.sh
```

See [M5 walkthrough](docs/m5-walkthrough.md), [M5 report](docs/m5-report.md), [M6 report](docs/m6-report.md), [M6.1 report](docs/m6.1-report.md), [M7.0 report](docs/m7.0-report.md), [M7.1 report](docs/m7.1-report.md), [M7.2 report](docs/m7.2-report.md), [M7.3 report](docs/m7.3-report.md), the [formal M7 report](docs/m7-report.md), the [M8 disposition contract](docs/m8-disposition-contract.md), the [M8.0 report](docs/m8.0-report.md), the [M8.1 report](docs/m8.1-report.md), and the [M8.2 report](docs/m8.2-report.md).
