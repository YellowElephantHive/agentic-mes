# M3 implementation report

Status: **PASS**

Recorded automatically after all M3 acceptance checks passed.

## Implemented

- Four strict, typed MES tools: work order, production, equipment downtime, and quality.
- Source-dataset ground-truth tool used by Track A and Track B.
- Allowlisted registry; model arguments cannot supply tenant IDs, factory IDs, credentials, or SQL.
- Trusted `AuthorizedToolContext` and per-call capability enforcement.
- Bounded runtime: 6 iterations, 8 tool calls, 10-second provider timeout, 5-second tool timeout, and 45-second overall limit.
- Deterministic scripted mock provider.
- Persisted `agent_runs` lifecycle and `agent_tool_calls` audits with arguments, authorization decision, structured result, and duration.
- Grounded final-answer generation from tool observations and evidence IDs.
- Unknown-tool denial and failed-run persistence.

## Acceptance evidence

The PostgreSQL integration test asked why Line A output dropped. The mock requested two production summaries, downtime, and quality. The grounded answer reported 1000 → 600 good units, listed machine failure/material shortage records, distinguished the QC sample rate, and stated that contributing hypotheses are not proof of causation.

The completed run persisted exactly four authorized tool calls. A separate `execute_raw_sql` request was rejected, audited as denied, and its run persisted as failed.

Commands passed:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
TEST_DATABASE_URL=... cargo test --package agent-core \
  --test runtime_integration -- --ignored --nocapture
```

Track A and Track B were rerun after M3 changes and both remained PASS.
