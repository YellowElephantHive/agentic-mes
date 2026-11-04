#!/usr/bin/env bash
set -euo pipefail

project_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_dir"

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 a|b" >&2
    exit 2
fi

track="$1"
database_url=${DATABASE_URL:-postgres://agentic_mes:agentic_mes@127.0.0.1:5432/agentic_mes}

case "$track" in
    a)
        ./scripts/fetch-evaluation-datasets.sh --track a
        DATABASE_URL="$database_url" cargo run --quiet --package factory-simulator -- \
            import-ai4i --csv data/evaluation/ai4i/ai4i2020.csv
        ;;
    b)
        ./scripts/fetch-evaluation-datasets.sh --track b
        DATABASE_URL="$database_url" cargo run --quiet --package factory-simulator -- \
            import-secom --data data/evaluation/secom/secom.data \
            --labels data/evaluation/secom/secom_labels.data
        ;;
    *)
        echo "Usage: $0 a|b" >&2
        exit 2
        ;;
esac

DATABASE_URL="$database_url" cargo run --quiet --package agent-core \
    --example evaluate_tracks -- --track "$track"
