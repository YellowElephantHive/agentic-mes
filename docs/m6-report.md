# M6 real-provider Agent Service report

Status: **PASS**

Recorded automatically on 2026-11-03 after the formal Agent API used a local Ollama model, all security/failure gates passed, and Track A and Track B independently remained PASS.

## Implemented

- Startup provider selection: `mock`, local `ollama`, or vendor-neutral `http`.
- `LLM_DATA_POLICY=local_only|external_approved`; external HTTP fails startup without explicit approval.
- `/readyz` reports database readiness, configured provider type, and data policy without secrets.
- Configurable bounded provider, tool, and overall runtime timeouts.
- Provider concurrency limit and consecutive-failure circuit breaker.
- Correct Ollama multi-turn history with assistant tool calls, arguments, and tool results.
- Rust rebuilds evidence exclusively from successfully executed tool observations; model-supplied evidence is ignored.
- The production-analysis API requires the exact baseline/current production calls and current downtime/quality calls. Premature or incomplete answers become `failed` with `incomplete_tool_evidence`.
- Agent runs persist `provider_name` and `data_policy` for audit inspection.

## Real-provider result

Provider: local Ollama `gemma4:e4b`, policy `local_only`.

Run `cd987179-e682-4369-92ae-ad91a3fc0ea6` completed through the formal `POST /api/v1/agent/runs` endpoint with exactly four authorized calls:

1. baseline production summary;
2. current production summary;
3. current equipment downtime;
4. current quality summary.

The answer reported 1,000 → 600 good units, 41.7 → 25.0 good units/hour, 360 minutes of unioned line downtime, machine-failure and material-shortage records, and a 2/100 QC sample defect result. Returned evidence was rebuilt by Rust and contained four production records, two downtime events, and one quality inspection.

## Failure-path evidence

- The initial real run hit the original 10-second provider bound and persisted `provider_timeout`.
- A later model repeated calls; the runtime stopped it at eight calls with `tool_call_limit`.
- A later model omitted current downtime; the new completeness gate prevents such a response from reaching the user.
- An automated scripted-provider test verifies incomplete evidence persists `failed/incomplete_tool_evidence`.
- An automated failing-provider test verifies the circuit opens after its configured consecutive-failure threshold without another upstream request.
- An external HTTP startup attempt under `local_only` was rejected before serving requests.

## Verification

- Formatting, clippy with warnings denied, workspace tests: PASS.
- PostgreSQL Agent runtime and HTTP integration tests: PASS.
- Provider retry, malformed-response, circuit-breaker and policy tests: PASS.
- Local Ollama formal API run and persisted provider/policy audit: PASS.
- Track A — UCI AI4I 2020: **PASS**.
- Track B — UCI SECOM: **PASS**.

M6 extends the executable MVP but does not make it production-certified. OIDC/JWT, distributed rate limiting, high availability, centralized secret management, and human-approved write workflows remain outside the current scope.
