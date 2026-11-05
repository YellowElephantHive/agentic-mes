#!/usr/bin/env sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

for command in docker jq; do
    command -v "$command" >/dev/null 2>&1 || {
        printf 'missing required command: %s\n' "$command" >&2
        exit 1
    }
done

pilot_project=${PILOT_COMPOSE_PROJECT:-agentic-mes-pilot}
postgres_user=${POSTGRES_USER:-agentic_mes}
postgres_db=${POSTGRES_DB:-agentic_mes}
timestamp=$(date -u +'%Y%m%dT%H%M%SZ')
output_dir=${PILOT_EXPORT_DIR:-artifacts/pilot/$timestamp}
mkdir -p "$output_dir"

query="
SELECT df.id AS feedback_id,
       df.created_at,
       COALESCE(df.trace_metadata->>'participant_code','') AS participant_code,
       COALESCE(df.trace_metadata->>'scenario_id','') AS scenario_id,
       CASE WHEN jsonb_typeof(df.trace_metadata->'task_elapsed_seconds')='number'
            THEN (df.trace_metadata->>'task_elapsed_seconds')::BIGINT END AS task_elapsed_seconds,
       df.evidence_rating,
       df.realism_rating,
       COALESCE(df.next_source,'') AS next_source,
       COALESCE(df.missing_factor,'') AS missing_factor,
       df.consent_for_research,
       df.run_id,
       df.proposal_id,
       df.plan_id
FROM domain_feedback df
ORDER BY df.created_at, df.id
"

docker compose --project-name "$pilot_project" exec -T db psql \
    --username "$postgres_user" --dbname "$postgres_db" --csv \
    --command "$query" >"$output_dir/feedback.csv"

docker compose --project-name "$pilot_project" exec -T db psql \
    --username "$postgres_user" --dbname "$postgres_db" --tuples-only --no-align \
    --command "SELECT COALESCE(jsonb_agg(to_jsonb(export_rows) ORDER BY created_at, feedback_id), '[]'::jsonb) FROM ($query) export_rows" \
    >"$output_dir/feedback.json"

jq --arg generated_at "$timestamp" '
  {
    contract_version: "m10.5.v1",
    generated_at_utc: $generated_at,
    timing_semantics: "browser_run_to_feedback",
    completed_tasks: length,
    unique_participant_codes: ([.[].participant_code | select(length > 0)] | unique | length),
    consented_for_research: ([.[] | select(.consent_for_research == true)] | length),
    evidence_ratings: (group_by(.evidence_rating) | map({key: .[0].evidence_rating, value: length}) | from_entries),
    realism_ratings: (group_by(.realism_rating) | map({key: .[0].realism_rating, value: length}) | from_entries),
    measured_elapsed_tasks: ([.[].task_elapsed_seconds | select(. != null)] | length),
    median_elapsed_seconds: ([.[].task_elapsed_seconds | select(. != null)] | sort | if length == 0 then null else .[length / 2 | floor] end),
    limitations: [
      "Elapsed time starts when Run investigation is pressed and ends when feedback is submitted.",
      "It is a browser-observed usability measure, not production labor savings.",
      "Participant codes are pseudonyms and are not verified identities."
    ]
  }
' "$output_dir/feedback.json" >"$output_dir/metrics.json"

printf '%s\n' \
    'Pilot feedback export: PASS' \
    "  CSV: $output_dir/feedback.csv" \
    "  JSON: $output_dir/feedback.json" \
    "  Metrics: $output_dir/metrics.json"
