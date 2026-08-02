# Optional GLM consolidation

GLM is optional. Raw multi-agent capture, SQLite evidence, startup context, and
deterministic search continue when no provider is configured or the provider is
offline. GLM only proposes typed curated memory from already-redacted evidence.

Add an opt-in provider block to the central `runtime/service.json`:

```json
{
  "schema_version": 2,
  "pipe_name": "\\\\.\\pipe\\agent-brain-v1",
  "consolidation": {
    "provider": "glm",
    "endpoint": "https://your-openai-compatible-endpoint/v1/chat/completions",
    "model": "your-glm-model",
    "api_key_env": "GLM_API_KEY",
    "timeout_ms": 30000,
    "max_retries": 2
  },
  "projects": []
}
```

Set the named environment variable for the service process. The key is read only
when making a request and is never persisted or logged. Evidence packets redact
credential assignments, common API-key forms, and high-entropy tokens first;
the ledger stores only redaction category and SHA-256 hash.

The client requests strict JSON. Local validation rejects unknown evidence or
supersession IDs, preferences, invalid timestamps, missing citations,
unsupported kinds, oversized fields, and invalid confidence. HTTP 429, 5xx,
connect failures, and timeouts retry within the configured bound. A remaining
failure returns the durable job to exponential backoff and never marks capture
unhealthy.
