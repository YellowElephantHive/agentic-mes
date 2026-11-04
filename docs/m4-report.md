# M4 implementation report

Status: **PASS**

Recorded automatically after the configurable HTTP adapter, failure lifecycle, contract tests, and real-model demo passed.

## Implemented

- Vendor-neutral HTTP tool-calling adapter.
- Native Ollama `/api/chat` adapter.
- Environment-based endpoint, model, timeout, retry, response-size, and optional bearer-secret configuration.
- Bounded 429/5xx retry with backoff.
- Strict response mapping: tool calls XOR final answer.
- Malformed, ambiguous, oversized, and unsuccessful responses fail closed.
- Provider secrets are private and omitted from Debug/log output.
- Deterministic mock remains the default CI provider.
- Runtime policy remains downstream of every provider; HTTP providers cannot bypass tool allowlisting, typed arguments, capability checks, or tenant/factory scope.
- Provider failures now persist `agent_runs.status=failed` and `error_code=provider_error`; provider timeouts persist `timed_out`.

## Verification

- Local HTTP contract server returned a transient 500, then a valid tool call; retry count and mapping were verified.
- Ambiguous answer-plus-tool response was rejected.
- PostgreSQL runtime test verified provider failure lifecycle persistence.
- Real Ollama `gemma4:e4b` returned `get_work_order` with the requested UUID through native structured tool calling.
- Track A and Track B regressions remained PASS.

No external API secret was required or fabricated.
