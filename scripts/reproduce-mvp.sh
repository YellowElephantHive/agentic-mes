#!/usr/bin/env sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

database_url=${DATABASE_URL:-postgres://agentic_mes:agentic_mes@127.0.0.1:5432/agentic_mes}
postgres_user=${POSTGRES_USER:-agentic_mes}
postgres_db=${POSTGRES_DB:-agentic_mes}
compose_project=${REPRODUCE_COMPOSE_PROJECT:-agentic-mes-reproduce}

compose() {
    docker compose --project-name "$compose_project" "$@"
}

cleanup() {
    compose down --volumes >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

# Preserve the normal development volume. The reproducibility run owns a
# separate disposable Compose project and database volume.
docker compose down >/dev/null 2>&1 || true
compose down --volumes >/dev/null 2>&1 || true
compose up --detach --wait db

DATABASE_URL="$database_url" cargo run --quiet --package factory-simulator -- \
    seed --seed 42 --days 30 --factory demo

identity_row=$(compose exec -T db psql \
    --username "$postgres_user" --dbname "$postgres_db" --tuples-only --no-align \
    --field-separator ' ' --command \
    "SELECT t.id, f.id, l.id FROM tenants t JOIN factories f ON f.tenant_id=t.id JOIN production_lines l ON l.factory_id=f.id WHERE f.slug='demo' AND l.name='Line A' LIMIT 1")
set -- $identity_row
tenant_id=$1
factory_id=$2
line_id=$3

curl --fail --silent --show-error \
    --header "x-dev-tenant-id: $tenant_id" \
    --header "x-dev-factory-id: $factory_id" \
    "http://127.0.0.1:8080/api/v1/production/summary?line_id=$line_id&start_at=2026-01-30T00%3A00%3A00Z&end_at=2026-01-31T00%3A00%3A00Z" \
    >/dev/null 2>&1 || {
        compose up --detach --build --wait api
        curl --fail --silent --show-error \
            --header "x-dev-tenant-id: $tenant_id" \
            --header "x-dev-factory-id: $factory_id" \
            "http://127.0.0.1:8080/api/v1/production/summary?line_id=$line_id&start_at=2026-01-30T00%3A00%3A00Z&end_at=2026-01-31T00%3A00%3A00Z"
    }

DATABASE_URL="$database_url" cargo run --quiet --package agent-core --example m5_walkthrough

DATABASE_URL="$database_url" ./scripts/evaluate-track.sh a
DATABASE_URL="$database_url" ./scripts/evaluate-track.sh b

printf '%s\n' 'M5 walkthrough: PASS' 'Track A: PASS' 'Track B: PASS'
