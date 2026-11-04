#!/usr/bin/env sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

for command in cargo curl docker; do
    command -v "$command" >/dev/null 2>&1 || {
        printf 'missing required command: %s\n' "$command" >&2
        exit 1
    }
done

postgres_port=${POSTGRES_PORT:-5432}
api_port=${API_PORT:-8080}
postgres_user=${POSTGRES_USER:-agentic_mes}
postgres_password=${POSTGRES_PASSWORD:-agentic_mes}
postgres_db=${POSTGRES_DB:-agentic_mes}
database_url="postgres://${postgres_user}:${postgres_password}@127.0.0.1:${postgres_port}/${postgres_db}"
console_url=${CONSOLE_URL:-http://127.0.0.1:${api_port}}

compose() {
    AGENT_PROVIDER=mock LLM_DATA_POLICY=local_only \
        POSTGRES_PORT="$postgres_port" API_PORT="$api_port" docker compose "$@"
}

compose up --detach --wait db
DATABASE_URL="$database_url" cargo run --quiet --package factory-simulator -- \
    seed --seed 42 --days 30 --factory demo >/dev/null
compose up --detach --build --wait api
curl --fail --silent "$console_url/readyz" >/dev/null

printf '%s\n' \
    'Agentic MES Operator Console: READY' \
    "Open: $console_url" \
    'Stop later with: docker compose down'
