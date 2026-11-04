# Mandatory dual-track evaluation contract

This contract supplements the implementation specification and is binding for all later milestones.

## Independent tracks

### Track A — Published synthetic manufacturing dataset

Use a published synthetic manufacturing dataset whose license, version, retrieval method, checksum, and field mapping are recorded. The adapter must keep original records immutable and attach source identifiers to normalized records. Dataset suitability and mappings must be verified; unrelated datasets must never be joined through invented identifiers.

### Track B — Real-world manufacturing dataset

Use a legitimately accessible real-world manufacturing dataset. Record its owner/source, version or snapshot date, usage restrictions, retrieval method, checksum when permitted, and adapter mapping. Sensitive source data must not be committed or sent to an external model without explicit authorization.

Track B is **not passed** until an actual eligible dataset is supplied or obtained and evaluated. Synthetic data, simulator output, or guessed values cannot stand in for it.

## Shared non-negotiable rules

Each track must have its own dataset-specific adapter, immutable raw-record boundary, provenance mapping, fixtures, ground-truth procedure, test command, result artifacts, and evaluation report. Adapters must expose missingness rather than invent, interpolate, or silently default manufacturing facts.

Agent tools may return only results calculated from authorized source records. Every result must include evidence identifiers, units, time bounds, and missing-data indicators. Tool selection, calculations, provenance completeness, authorization denial, malformed calls, and prompt-injection resistance must be reproducibly tested.

## Passing gate

A track passes only when its documented acceptance thresholds all pass in a clean, reproducible run. Reports are generated separately:

- `reports/track-a/REPORT.md`
- `reports/track-b/REPORT.md`

The overall MVP status remains incomplete unless **both** Track A and Track B pass independently. Reports must say `NOT EVALUATED` or `FAILED` where applicable; they must never infer a pass from the other track.

M1 selected UCI AI4I 2020 for Track A and UCI SECOM for Track B. Their source, license, checksums, mappings, and limitations are frozen in [dataset provenance](dataset-provenance.md). Agent task definitions, metrics, and passing thresholds must be frozen before M3 evaluation begins; adapter smoke tests alone do not pass either track.
