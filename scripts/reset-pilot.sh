#!/usr/bin/env sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

if [ "${PILOT_CONFIRM_RESET:-}" != "RESET" ]; then
    printf '%s\n' \
        'Reset refused: set PILOT_CONFIRM_RESET=RESET.' \
        'The dedicated pilot database is exported before its volume is removed.' >&2
    exit 2
fi

pilot_project=${PILOT_COMPOSE_PROJECT:-agentic-mes-pilot}
pilot_volume="${pilot_project}_postgres-data"
if docker volume inspect "$pilot_volume" >/dev/null 2>&1; then
    POSTGRES_PORT=${PILOT_POSTGRES_PORT:-5432} \
        docker compose --project-name "$pilot_project" up --detach --wait db
    ./scripts/export-pilot-feedback.sh
fi

docker compose --project-name "$pilot_project" down --volumes
./scripts/start-pilot.sh
