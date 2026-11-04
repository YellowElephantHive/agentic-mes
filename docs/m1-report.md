# M1 implementation report

Status: **PASS**

This status applies only to M1 — MES core and data. It does not mean either agent evaluation track or the overall MVP passed.

## Implemented

- SQLx PostgreSQL schema with tenant/factory/line composite foreign keys, indexes, checks, work-order transition trigger, audit tables, and immutable dataset provenance tables.
- Database-independent domain invariants for quantities, quality samples, time ranges, and work-order transitions.
- PostgreSQL repository for repeatable transactional seed replacement and tenant-scoped production aggregates.
- Deterministic simulator with fixed dates, UUID v5 identities, 120 work orders, and documented scenarios.
- Dataset-specific AI4I and SECOM adapters, SHA-256 manifests, original payload retention, normalized observations, and explicit missingness.
- Reproducible official-dataset retrieval with archive checksum verification.

## Verification performed

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
TEST_DATABASE_URL=... cargo test --package factory-simulator \
  --test postgres_integration -- --ignored --nocapture
```

The database integration test applies migrations, seeds twice, verifies 120 work orders, verifies Line A 1,000 → 600 ground truth, and confirms a wrong tenant scope returns zero records. Full official adapter runs parsed and persisted AI4I 10,000/10,000 and SECOM 1,567/1,567 records.

## Boundary

M2 routes, development identity, HTTP authorization, agent tools, LLM providers, and Track A/B agent evaluation are not implemented in M1.
