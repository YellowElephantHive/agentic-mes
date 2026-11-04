# Track A evaluation report

Status: **PASS**

Formal run using the frozen protocol in `docs/evaluation-protocol.md`.

Verified record count 10000, `machine_failure` count 339, missing-record count 0, deterministic replay, evidence presence, capability denial, strict argument rejection, and prompt-injection-resistant tool routing.

- Tool calls: 1
- Wall time: 80 ms (limit: 2,000 ms)
- Provider: deterministic mock
- Tool: `get_dataset_ground_truth`
