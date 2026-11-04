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
demo_exists=$(compose exec -T db psql \
    --username "$postgres_user" --dbname "$postgres_db" --tuples-only --no-align \
    --command "SELECT CASE WHEN to_regclass('public.factories') IS NULL THEN 0 ELSE (SELECT COUNT(*) FROM factories WHERE slug='demo') END" \
    2>/dev/null || printf '0')
if [ "$(printf '%s' "$demo_exists" | tr -d '[:space:]')" = "0" ]; then
    DATABASE_URL="$database_url" cargo run --quiet --package factory-simulator -- \
        seed --seed 42 --days 30 --factory demo >/dev/null
else
    printf '%s\n' 'Reusing existing demo factory and case history'
fi

database_row=$(compose exec -T db psql \
    --username "$postgres_user" --dbname "$postgres_db" --tuples-only --no-align \
    --field-separator ' ' --command \
    "SELECT t.id,f.id,m.id FROM tenants t JOIN factories f ON f.tenant_id=t.id JOIN tenant_memberships m ON m.tenant_id=t.id JOIN external_identities e ON e.user_id=m.user_id WHERE f.slug='demo' AND e.subject='dev-user' LIMIT 1")
set -- $database_row
tenant_id=$1
factory_id=$2
creator_membership_id=$3

reviewer_sql="
INSERT INTO users (id,display_name,status) VALUES ('97e8db83-d5c2-5cef-bdc6-b25bb4d217b9','M9 Demo Reviewer','active') ON CONFLICT DO NOTHING;
INSERT INTO external_identities (id,user_id,issuer,subject) VALUES ('91223e0f-ed7e-5c6c-9418-70004be9ce71','97e8db83-d5c2-5cef-bdc6-b25bb4d217b9','urn:agentic-mes:dev','m9-demo-reviewer') ON CONFLICT DO NOTHING;
INSERT INTO tenant_memberships (id,user_id,tenant_id,status,all_factories) VALUES ('8755d006-e1a5-5c86-a9b2-f67bcf4ecf56','97e8db83-d5c2-5cef-bdc6-b25bb4d217b9','$tenant_id','active',false) ON CONFLICT DO NOTHING;
INSERT INTO tenant_membership_roles (membership_id,role) VALUES ('8755d006-e1a5-5c86-a9b2-f67bcf4ecf56','supervisor') ON CONFLICT DO NOTHING;
INSERT INTO membership_factory_scopes (membership_id,tenant_id,factory_id) VALUES ('8755d006-e1a5-5c86-a9b2-f67bcf4ecf56','$tenant_id','$factory_id') ON CONFLICT DO NOTHING;
INSERT INTO tenant_membership_roles (membership_id,role) VALUES ('$creator_membership_id','supervisor') ON CONFLICT DO NOTHING;
"
compose exec -T db psql --username "$postgres_user" --dbname "$postgres_db" \
    --set ON_ERROR_STOP=1 --command "$reviewer_sql" >/dev/null

compose up --detach --build --wait api
curl --fail --silent "$console_url/readyz" >/dev/null

printf '%s\n' \
    'Agentic MES Operator Console: READY' \
    "Open: $console_url" \
    'M10 reviewer: m9-demo-reviewer (local development only)' \
    'Includes authorized work queue, scenario presets, and expert annotations' \
    'Stop later with: docker compose down'
