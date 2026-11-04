# HTTP provider contract

## Vendor-neutral adapter

The configured endpoint receives:

```json
{
  "model": "configured-model",
  "system": "policy text",
  "message": "user question",
  "tools": [{"name": "tool_name", "parameters": {}}],
  "observations": []
}
```

It must return exactly one of:

```json
{"tool_calls":[{"call_id":"1","tool_name":"get_work_order","arguments":{}}]}
```

```json
{"answer":"source-grounded answer","evidence":["record:id"]}
```

Responses containing both forms, neither form, malformed JSON, non-success status, or more than 1 MiB are rejected. HTTP 429 and 5xx responses use bounded retries and backoff. The optional bearer key is private and is never included in Debug output.

## Native Ollama adapter

The Ollama adapter maps the same runtime request to native `POST /api/chat` messages and function-tool definitions with `stream=false`. Ollama tool calls are normalized into runtime `ModelToolCall` values. Model-produced names and arguments remain untrusted and must pass the M3 registry, typed deserialization, capability check, and tenant/factory scope.

Real-model smoke command:

```bash
OLLAMA_MODEL=gemma4:e4b \
  cargo run --package llm-provider --example ollama_demo
```
