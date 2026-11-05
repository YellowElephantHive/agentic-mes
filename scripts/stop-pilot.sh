#!/usr/bin/env sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

pilot_project=${PILOT_COMPOSE_PROJECT:-agentic-mes-pilot}
docker compose --project-name "$pilot_project" down
printf 'Guided pilot stopped; the dedicated database volume was retained.\n'
