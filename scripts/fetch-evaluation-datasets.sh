#!/usr/bin/env bash
set -euo pipefail

project_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
output_dir="$project_dir/data/evaluation"
mkdir -p "$output_dir/ai4i" "$output_dir/secom"

ai4i_url='https://archive.ics.uci.edu/static/public/601/ai4i+2020+predictive+maintenance+dataset.zip'
secom_url='https://archive.ics.uci.edu/static/public/179/secom.zip'
ai4i_zip="$output_dir/ai4i.zip"
secom_zip="$output_dir/secom.zip"

curl --fail --location --silent --show-error "$ai4i_url" --output "$ai4i_zip"
curl --fail --location --silent --show-error "$secom_url" --output "$secom_zip"

checksum() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

expected_ai4i='f601f14294bcf190f9d720676b7f0aea46a26cde9ab8ebc7b4f8174d9d26b252'
expected_secom='eea568baf3c2229096d7d294cf0b096b5502bd96d92c0b80a65b84714059be8e'
actual_ai4i=$(checksum "$ai4i_zip")
actual_secom=$(checksum "$secom_zip")

if [[ "$actual_ai4i" != "$expected_ai4i" ]]; then
    echo "AI4I archive checksum mismatch: $actual_ai4i" >&2
    exit 1
fi
if [[ "$actual_secom" != "$expected_secom" ]]; then
    echo "SECOM archive checksum mismatch: $actual_secom" >&2
    exit 1
fi

unzip -oq "$ai4i_zip" -d "$output_dir/ai4i"
unzip -oq "$secom_zip" -d "$output_dir/secom"
echo "Verified datasets written under $output_dir"
