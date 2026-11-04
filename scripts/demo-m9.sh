#!/usr/bin/env sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

for command in cargo curl docker jq; do
    command -v "$command" >/dev/null 2>&1 || {
        printf 'missing required command: %s\n' "$command" >&2
        exit 1
    }
done

demo_db_port=${DEMO_POSTGRES_PORT:-55432}
demo_api_port=${DEMO_API_PORT:-58080}
demo_project=${DEMO_COMPOSE_PROJECT:-agentic-mes-demo}
database_url=${DEMO_DATABASE_URL:-postgres://agentic_mes:agentic_mes@127.0.0.1:${demo_db_port}/agentic_mes}
api_url=${DEMO_API_URL:-http://127.0.0.1:${demo_api_port}}
output_file=${DEMO_OUTPUT:-artifacts/m9.7-demo.json}
manage_database=${DEMO_MANAGE_DATABASE:-1}
temporary_dir=$(mktemp -d)
api_pid=

compose() {
    POSTGRES_PORT="$demo_db_port" API_PORT="$demo_api_port" \
        docker compose --project-name "$demo_project" "$@"
}

cleanup() {
    if [ -n "$api_pid" ]; then
        kill "$api_pid" >/dev/null 2>&1 || true
        wait "$api_pid" >/dev/null 2>&1 || true
    fi
    if [ "$manage_database" = "1" ]; then
        compose down --volumes >/dev/null 2>&1 || true
    fi
    rm -rf "$temporary_dir"
}
trap cleanup EXIT INT TERM

if [ "$manage_database" = "1" ]; then
    compose down --volumes >/dev/null 2>&1 || true
    compose up --detach --wait db
fi

DATABASE_URL="$database_url" cargo run --quiet --package factory-simulator -- \
    seed --seed 42 --days 30 --factory demo >/dev/null

if [ "$manage_database" = "1" ]; then
    database_row=$(compose exec -T db psql \
        --username agentic_mes --dbname agentic_mes --tuples-only --no-align \
        --field-separator ' ' --command \
        "SELECT t.id,f.id,l.id,m.id FROM tenants t JOIN factories f ON f.tenant_id=t.id JOIN production_lines l ON l.factory_id=f.id JOIN tenant_memberships m ON m.tenant_id=t.id JOIN external_identities e ON e.user_id=m.user_id WHERE f.slug='demo' AND l.name='Line A' AND e.subject='dev-user' LIMIT 1")
else
    command -v psql >/dev/null 2>&1 || {
        printf 'psql is required when DEMO_MANAGE_DATABASE=0\n' >&2
        exit 1
    }
    database_row=$(psql "$database_url" --tuples-only --no-align \
        --field-separator ' ' --command \
        "SELECT t.id,f.id,l.id,m.id FROM tenants t JOIN factories f ON f.tenant_id=t.id JOIN production_lines l ON l.factory_id=f.id JOIN tenant_memberships m ON m.tenant_id=t.id JOIN external_identities e ON e.user_id=m.user_id WHERE f.slug='demo' AND l.name='Line A' AND e.subject='dev-user' LIMIT 1")
fi
set -- $database_row
tenant_id=$1
factory_id=$2
line_id=$3
creator_membership_id=$4

reviewer_sql="
INSERT INTO users (id,display_name,status) VALUES ('97e8db83-d5c2-5cef-bdc6-b25bb4d217b9','M9 Demo Reviewer','active');
INSERT INTO external_identities (id,user_id,issuer,subject) VALUES ('91223e0f-ed7e-5c6c-9418-70004be9ce71','97e8db83-d5c2-5cef-bdc6-b25bb4d217b9','urn:agentic-mes:dev','m9-demo-reviewer');
INSERT INTO tenant_memberships (id,user_id,tenant_id,status,all_factories) VALUES ('8755d006-e1a5-5c86-a9b2-f67bcf4ecf56','97e8db83-d5c2-5cef-bdc6-b25bb4d217b9','$tenant_id','active',false);
INSERT INTO tenant_membership_roles (membership_id,role) VALUES ('8755d006-e1a5-5c86-a9b2-f67bcf4ecf56','supervisor');
INSERT INTO membership_factory_scopes (membership_id,tenant_id,factory_id) VALUES ('8755d006-e1a5-5c86-a9b2-f67bcf4ecf56','$tenant_id','$factory_id');
INSERT INTO tenant_membership_roles (membership_id,role) VALUES ('$creator_membership_id','supervisor') ON CONFLICT DO NOTHING;
"
if [ "$manage_database" = "1" ]; then
    compose exec -T db psql --username agentic_mes --dbname agentic_mes \
        --set ON_ERROR_STOP=1 --command "$reviewer_sql" >/dev/null
else
    psql "$database_url" --set ON_ERROR_STOP=1 --command "$reviewer_sql" >/dev/null
fi

APP_ENV=development \
AUTH_PROVIDER=dev_headers \
AGENT_PROVIDER=mock \
LLM_DATA_POLICY=local_only \
APP_BIND_ADDR="127.0.0.1:${demo_api_port}" \
DATABASE_URL="$database_url" \
cargo run --quiet --package mes-api >"$temporary_dir/api.log" 2>&1 &
api_pid=$!

ready=0
attempt=0
while [ "$attempt" -lt 60 ]; do
    if curl --fail --silent "$api_url/readyz" >/dev/null 2>&1; then
        ready=1
        break
    fi
    if ! kill -0 "$api_pid" >/dev/null 2>&1; then
        cat "$temporary_dir/api.log" >&2
        exit 1
    fi
    attempt=$((attempt + 1))
    sleep 1
done
if [ "$ready" != "1" ]; then
    cat "$temporary_dir/api.log" >&2
    printf 'API did not become ready\n' >&2
    exit 1
fi

console_html=$(curl --fail --silent --show-error "$api_url/")
printf '%s' "$console_html" | grep -q 'Agentic MES Operator Console'
curl --fail --silent --show-error "$api_url/assets/operator-console.css" | grep -q -- '--color-signal'
curl --fail --silent --show-error "$api_url/assets/operator-console.js" | grep -q 'runInvestigation'
printf '%s' "$console_html" | grep -q 'Human-controlled lifecycle'
curl --fail --silent --show-error "$api_url/assets/operator-console.js" | grep -q 'reviewProposal'
if curl --fail --silent --show-error "$api_url/assets/operator-console.js" | grep -Eq '/execute|execute_action|executePlan'; then
    printf 'operator console must not expose an execution route\n' >&2
    exit 1
fi

creator_headers="x-dev-tenant-id: $tenant_id"
factory_header="x-dev-factory-id: $factory_id"
json_header='content-type: application/json'

agent_run=$(curl --fail --silent --show-error --request POST "$api_url/api/v1/agent/runs" \
    --header "$json_header" --header "$creator_headers" --header "$factory_header" \
    --data "{\"agent_id\":\"production-analyst\",\"factory_id\":\"$factory_id\",\"line_id\":\"$line_id\",\"start_at\":\"2026-01-30T00:00:00Z\",\"end_at\":\"2026-01-31T00:00:00Z\",\"message\":\"Why did Line A output drop?\"}")
printf '%s' "$agent_run" | jq -e '.data.status == "completed" and (.data.evidence | length >= 3)' >/dev/null
run_id=$(printf '%s' "$agent_run" | jq -r '.data.run_id')

proposal=$(curl --fail --silent --show-error --request POST \
    "$api_url/api/v1/agent/runs/$run_id/disposition-proposals" \
    --header "$json_header" --header "$creator_headers" --header "$factory_header" \
    --header 'idempotency-key: m9.7-proposal-create' \
    --data '{"proposal_type":"maintenance_investigation"}')
printf '%s' "$proposal" | jq -e '.data.status == "draft" and .data.evidence_state == "supported"' >/dev/null
proposal_id=$(printf '%s' "$proposal" | jq -r '.data.id')

proposal_submitted=$(curl --fail --silent --show-error --request POST \
    "$api_url/api/v1/disposition-proposals/$proposal_id/submit" \
    --header "$json_header" --header "$creator_headers" --header "$factory_header" \
    --header 'idempotency-key: m9.7-proposal-submit' --header 'if-match: 1' --data '{}')
printf '%s' "$proposal_submitted" | jq -e '.data.proposal.status == "pending_review"' >/dev/null

proposal_approved=$(curl --fail --silent --show-error --request POST \
    "$api_url/api/v1/disposition-proposals/$proposal_id/reviews" \
    --header "$json_header" --header "$creator_headers" --header "$factory_header" \
    --header 'x-dev-user-id: m9-demo-reviewer' \
    --header 'idempotency-key: m9.7-proposal-review' --header 'if-match: 2' \
    --data '{"decision":"approved","rationale":"Evidence supports a bounded human investigation."}')
printf '%s' "$proposal_approved" | jq -e '.data.proposal.status == "review_approved"' >/dev/null

action_plan=$(curl --fail --silent --show-error --request POST \
    "$api_url/api/v1/disposition-proposals/$proposal_id/action-plans" \
    --header "$json_header" --header "$creator_headers" --header "$factory_header" \
    --header 'idempotency-key: m9.7-plan-create' --data '{}')
printf '%s' "$action_plan" | jq -e '.data.status == "draft" and .data.compiler_policy_version == "m9.2.v1"' >/dev/null
plan_id=$(printf '%s' "$action_plan" | jq -r '.data.id')

validated=$(curl --fail --silent --show-error --request POST \
    "$api_url/api/v1/action-plans/$plan_id/validate" \
    --header "$json_header" --header "$creator_headers" --header "$factory_header" \
    --header 'idempotency-key: m9.7-plan-validate' --header 'if-match: 1' --data '{}')
printf '%s' "$validated" | jq -e '.data.plan.status == "validated" and (.data.snapshot.snapshot_hash | length == 64)' >/dev/null

override_body="$temporary_dir/seed-override.json"
override_status=$(curl --silent --show-error --output "$override_body" --write-out '%{http_code}' \
    --request POST "$api_url/api/v1/action-plans/$plan_id/simulate" \
    --header "$json_header" --header "$creator_headers" --header "$factory_header" \
    --header 'idempotency-key: m9.7-seed-override' --header 'if-match: 2' \
    --data '{"deterministic_seed":7}')
[ "$override_status" = "422" ] || {
    cat "$override_body" >&2
    printf 'expected seed override to fail with 422, received %s\n' "$override_status" >&2
    exit 1
}

simulated=$(curl --fail --silent --show-error --request POST \
    "$api_url/api/v1/action-plans/$plan_id/simulate" \
    --header "$json_header" --header "$creator_headers" --header "$factory_header" \
    --header 'idempotency-key: m9.7-plan-simulate' --header 'if-match: 2' --data '{}')
printf '%s' "$simulated" | jq -e '.data.plan.status == "simulated" and .data.simulation.deterministic_seed == 9400001 and .data.simulation.protected_state_unchanged == true' >/dev/null

plan_submitted=$(curl --fail --silent --show-error --request POST \
    "$api_url/api/v1/action-plans/$plan_id/submit" \
    --header "$json_header" --header "$creator_headers" --header "$factory_header" \
    --header 'idempotency-key: m9.7-plan-submit' --header 'if-match: 3' --data '{}')
printf '%s' "$plan_submitted" | jq -e '.data.plan.status == "pending_approval"' >/dev/null

self_review_body="$temporary_dir/self-review.json"
self_review_status=$(curl --silent --show-error --output "$self_review_body" --write-out '%{http_code}' \
    --request POST "$api_url/api/v1/action-plans/$plan_id/reviews" \
    --header "$json_header" --header "$creator_headers" --header "$factory_header" \
    --header 'idempotency-key: m9.7-self-review' --header 'if-match: 4' \
    --data '{"decision":"approved","rationale":"This must be denied."}')
[ "$self_review_status" = "403" ] || {
    cat "$self_review_body" >&2
    printf 'expected creator self-review to fail with 403, received %s\n' "$self_review_status" >&2
    exit 1
}

plan_approved=$(curl --fail --silent --show-error --request POST \
    "$api_url/api/v1/action-plans/$plan_id/reviews" \
    --header "$json_header" --header "$creator_headers" --header "$factory_header" \
    --header 'x-dev-user-id: m9-demo-reviewer' \
    --header 'idempotency-key: m9.7-plan-review' --header 'if-match: 4' \
    --data '{"decision":"approved","rationale":"Dry-run evidence supports this non-executable investigation plan."}')
printf '%s' "$plan_approved" | jq -e '.data.plan.status == "plan_approved" and .data.plan.version == 5' >/dev/null

mkdir -p "$(dirname "$output_file")"
jq -n \
    --arg run_id "$run_id" \
    --arg proposal_id "$proposal_id" \
    --arg plan_id "$plan_id" \
    --arg answer "$(printf '%s' "$agent_run" | jq -r '.data.answer')" \
    --arg snapshot_hash "$(printf '%s' "$validated" | jq -r '.data.snapshot.snapshot_hash')" \
    --arg simulation_input_hash "$(printf '%s' "$simulated" | jq -r '.data.simulation.canonical_input_hash')" \
    --arg simulation_output_hash "$(printf '%s' "$simulated" | jq -r '.data.simulation.canonical_output_hash')" \
    '{milestone:"M9.7",result:"PASS",run_id:$run_id,proposal_id:$proposal_id,plan_id:$plan_id,agent_answer:$answer,proposal_status:"review_approved",plan_status:"plan_approved",snapshot_hash:$snapshot_hash,simulation_input_hash:$simulation_input_hash,simulation_output_hash:$simulation_output_hash,fail_closed:{client_seed_override_http_status:422,creator_self_review_http_status:403},execution_authority:false}' \
    >"$output_file"

printf '%s\n' \
    'M9.7 demo: PASS' \
    "  proposal: review_approved ($proposal_id)" \
    "  plan: plan_approved but non-executable ($plan_id)" \
    '  fail-closed: seed override=422, creator self-review=403' \
    "  artifact: $output_file"
