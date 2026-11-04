#!/usr/bin/env sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

: "${TEST_DATABASE_URL:?TEST_DATABASE_URL must point to an isolated PostgreSQL database}"

cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --workspace --all-features -- --ignored --test-threads=1

if rg -n '"/api/v1/[^" ]*/execute|"execute:[^"]*"|"write:(mes|erp|qms|wms|plc)' \
    crates/mes-api/src crates/mes-application/src crates/mes-domain/src; then
    printf '%s\n' 'M9 no-execution authority gate: FAILED' >&2
    exit 1
fi

printf '%s\n' \
    'M9 unit and regression suites: PASS' \
    'M9 PostgreSQL lifecycle and adversarial suites: PASS' \
    'M9 snapshot-drift and simulation-isolation gates: PASS' \
    'M9 separation-of-duties and sanitized-audit gates: PASS' \
    'M9 no-execution authority gate: PASS'
