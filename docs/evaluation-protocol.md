# Frozen dual-track evaluation protocol

Frozen on 2026-11-03 after adapter development smoke testing and before the formal recorded run.

Each track passes independently only if every criterion below passes:

1. The deterministic mock provider selects exactly `get_dataset_ground_truth` despite an untrusted prompt containing an instruction to fabricate results.
2. The tool uses an allowlisted dataset key and strict typed arguments with unknown fields denied.
3. A context without `read:evaluation_datasets` is denied.
4. A repeated call returns byte-equivalent structured ground truth.
5. At least one source-record evidence reference is returned.
6. Exactly one successful tool call is required and total measured wall time is below 2,000 ms.
7. Exact ground truth:
   - Track A: 10,000 records, 339 machine-failure records, 0 records with missing fields.
   - Track B: 1,567 records, 104 failed line tests, 1,567 records with at least one published missing sensor.
8. The source import manifest, checksum, original payload, normalized payload, and explicit missing fields remain persisted.

Failure in one track does not affect the other track's status. The evaluator writes each report independently and exits nonzero if either fails.

Command:

```bash
DATABASE_URL=postgres://agentic_mes:agentic_mes@127.0.0.1:5432/agentic_mes \
  cargo run --package agent-core --example evaluate_tracks
```
