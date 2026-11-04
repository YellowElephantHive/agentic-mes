#!/usr/bin/env bash
set -euo pipefail

project_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
output_dir="$project_dir/data/evaluation"

usage() {
    echo "Usage: $0 [--track a|b|all]" >&2
}

track="all"
if [[ $# -gt 0 ]]; then
    if [[ $# -ne 2 || "$1" != "--track" ]]; then
        usage
        exit 2
    fi
    track="$2"
fi

if [[ "$track" != "a" && "$track" != "b" && "$track" != "all" ]]; then
    usage
    exit 2
fi

ai4i_url='https://archive.ics.uci.edu/static/public/601/ai4i+2020+predictive+maintenance+dataset.zip'
secom_url='https://archive.ics.uci.edu/static/public/179/secom.zip'
ai4i_zip="$output_dir/ai4i.zip"
secom_zip="$output_dir/secom.zip"
expected_ai4i='f601f14294bcf190f9d720676b7f0aea46a26cde9ab8ebc7b4f8174d9d26b252'
expected_secom='eea568baf3c2229096d7d294cf0b096b5502bd96d92c0b80a65b84714059be8e'

checksum() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

fetch_and_extract() {
    local label="$1"
    local url="$2"
    local archive="$3"
    local expected="$4"
    local destination="$5"
    local temporary="${archive}.tmp"

    mkdir -p "$destination"
    rm -f "$temporary"
    curl --fail --location --silent --show-error "$url" --output "$temporary"

    local actual
    actual=$(checksum "$temporary")
    if [[ "$actual" != "$expected" ]]; then
        rm -f "$temporary"
        echo "$label archive checksum mismatch: $actual" >&2
        exit 1
    fi

    mv "$temporary" "$archive"
    unzip -oq "$archive" -d "$destination"
    echo "$label dataset: PASS (checksum verified)"
}

if [[ "$track" == "a" || "$track" == "all" ]]; then
    fetch_and_extract \
        "Track A / AI4I" "$ai4i_url" "$ai4i_zip" "$expected_ai4i" "$output_dir/ai4i"
fi

if [[ "$track" == "b" || "$track" == "all" ]]; then
    fetch_and_extract \
        "Track B / SECOM" "$secom_url" "$secom_zip" "$expected_secom" "$output_dir/secom"
fi

echo "Verified requested datasets written under $output_dir"
