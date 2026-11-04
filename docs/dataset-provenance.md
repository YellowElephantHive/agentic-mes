# Dataset provenance and mapping freeze

Snapshot date: **2026-11-03**

These datasets are evaluation inputs, not relational MES records. Neither adapter invents work orders, machines, line IDs, production counts, downtime, or causal links.

## Track A — UCI AI4I 2020

- Dataset: AI4I 2020 Predictive Maintenance Dataset
- UCI ID / DOI: 601 / `10.24432/C5HS5C`
- Classification: published synthetic predictive-maintenance observations
- License: CC BY 4.0
- Official source: `https://archive.ics.uci.edu/dataset/601/ai4i+2020+predictive+maintenance+dataset`
- Archive SHA-256: `f601f14294bcf190f9d720676b7f0aea46a26cde9ab8ebc7b4f8174d9d26b252`
- Extracted `ai4i2020.csv` SHA-256: `dc6630cd9b1f0f853922fad78a1b6436570d3f1ec863f1dd5c4340ac56bc8a8e`
- Expected rows: 10,000

The adapter maps temperatures, rotational speed, torque, tool wear, machine-failure flag, and five published failure-mode flags with their stated units. Original CSV field strings are retained in `raw_payload`; normalization never converts observations into MES events.

## Track B — UCI SECOM

- Dataset: SECOM
- UCI ID / DOI: 179 / `10.24432/C54305`
- Classification: real semiconductor manufacturing process observations
- License: CC BY 4.0
- Official source: `https://archive.ics.uci.edu/dataset/179/secom`
- Archive SHA-256: `eea568baf3c2229096d7d294cf0b096b5502bd96d92c0b80a65b84714059be8e`
- Combined adapter-input SHA-256: `54d323b79c8b8a282675253cf81dd8313b4cf5f601af85f98786e3adaaf9a9d0`
- Expected rows: 1,567

The combined checksum is SHA-256 over `secom.data`, one zero byte, then `secom_labels.data`. The adapter preserves each original data line, label line, tokens, quoted timestamp, and `-1`/`1` label. It exposes `NaN` as JSON `null` plus an explicit `missing_fields` entry. It maps `-1` to `pass` and `1` to `fail`, matching the dataset documentation; it does not assign semantics to anonymous sensor columns.

## Retrieval

Run `scripts/fetch-evaluation-datasets.sh` for both datasets, or select one with
`--track a` or `--track b`. A checksum mismatch stops extraction. Raw datasets
are gitignored and remain outside source control.
