#!/usr/bin/env sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

export COMPOSE_PROJECT_NAME=${PILOT_COMPOSE_PROJECT:-agentic-mes-pilot}
export POSTGRES_PORT=${PILOT_POSTGRES_PORT:-5432}
export API_PORT=${PILOT_API_PORT:-8080}

./scripts/run-console.sh

printf '%s\n' \
    'Guided pilot: READY' \
    "Guide: $project_dir/docs/m10.5-guided-pilot.md" \
    'Use pseudonymous participant codes only; do not enter names or email addresses.'
