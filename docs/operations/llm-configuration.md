# Optional LLM consolidation

Consolidation is optional. Raw multi-agent capture, SQLite evidence, startup context, and
deterministic search continue when no provider is configured or the provider is
offline. The provider only proposes typed curated memory from already-redacted evidence.

## The provider

The adapter is `crates/brain-context/src/llm_client.rs`, and it is the only code in the
workspace that knows a provider exists. It speaks the **OpenAI Responses API** over HTTPS, so
any endpoint offering that interface can serve it. Nothing above the adapter — consolidation,
`brain revise`, `brain synthesize` — names a vendor or a model.

The configured default is **GPT‑5.6 Luna through OpenCode Go**:

| Setting | Value |
|---|---|
| Base URL | `https://opencode.ai/zen/go/v1` |
| Model | `gpt-5.6-luna` |
| Request path | `POST {base URL}/responses` |
| Auth | `Authorization: Bearer <key>` |

**Prefer the API root for `base_url`.** The client appends `/responses` itself. A complete
value that already ends in `/responses` is also accepted and not appended to twice; a
value ending in `/chat/completions` — the shape older configuration used — is **refused**,
because appending to it silently builds `/chat/completions/responses` and the resulting 404
reads like a provider outage rather than a configuration mistake. Both cases are pinned in
`crates/brain-context/tests/llm_contract.rs`.

## Configuration

Every field has a default, so the smallest working block names only the provider:

```json
{
  "schema_version": 2,
  "pipe_name": "\\\\.\\pipe\\agent-brain-v1",
  "consolidation": {
    "provider": "llm"
  },
  "projects": []
}
```

The full form, with the defaults written out:

```json
{
  "consolidation": {
    "provider": "llm",
    "base_url": "https://opencode.ai/zen/go/v1",
    "model": "gpt-5.6-luna",
    "api_key_env": "LLM_API_KEY",
    "timeout_ms": 30000,
    "max_retries": 2
  }
}
```

### Environment

| Variable | Meaning |
|---|---|
| `LLM_API_KEY` | **The key itself.** Read by name at request time; never stored in any file this repository tracks. |
| `LLM_BASE_URL` | Optional. Overrides `base_url`. |
| `LLM_MODEL` | Optional. Overrides `model`. |

`LLM_BASE_URL` and `LLM_MODEL` take precedence over `service.json`, and an *empty* variable is
treated as unset rather than as an override — a cleared value on a deployment platform must not
replace a working configured one. `api_key_env` names which variable holds the key, so a
deployment that must use a different name changes that one field rather than every call site.

Resolution happens in exactly one place, `ConsolidationProviderConfig::resolve` in
`crates/brain-service/src/config.rs`, and every consumer builds its client through
`ConsolidationProviderConfig::client`. Missing configuration fails there with a message naming
the setting to supply. **No error, log line or panic ever carries the key's value** — only the
name of the variable consulted and whether it resolved.

Set `LLM_API_KEY` for the *service* process. `AgentBrain.Service` runs under Task Scheduler in
its own environment, so a key readable in an interactive shell is not necessarily readable
there; `brain dashboard` reports whether the name resolves in the process it runs in, and says
so rather than claiming a verdict. A deferral logged as HTTP 401 is the signal that the service
cannot see it, where a 429 means quota rather than configuration.

## A stale provider block disables consolidation, it does not stop the service

A `consolidation` block this build cannot read — one naming a provider that no longer exists,
left behind by a provider migration — is **ignored with a warning**, and consolidation is the
only thing that stops. Failing the whole load would take the service, every CLI command and the
dashboard down with it, for a feature that is optional and whose absence is a supported, tested
state. The warning names the provider it could not read.

## Requests and responses

The request is a Responses API call: the consolidation rules travel as `instructions`, the
redacted evidence packet as `input`, and JSON is constrained through `text.format`. `store` is
`false` — evidence packets come from private transcripts and nothing here needs the provider to
retain them between calls.

Responses are read from the `output` array, which is a **list of items**: the assistant message
is searched for rather than indexed, because a reasoning item precedes it whenever the model
emits one. A response whose `status` is not `completed` is reported as the truncation it is,
not as a schema mismatch.

Local validation is unchanged and remains the authority: it rejects unknown evidence or
supersession IDs, preferences, invalid timestamps, missing citations, unsupported kinds,
oversized fields, and invalid confidence — per memory, so one bad proposal does not discard the
sound ones beside it. HTTP 429, 5xx, connect failures, and timeouts retry within the configured
bound. If availability or credentials still prevent a call, the durable job is deferred for the
lease duration without consuming an attempt. Only packet/output failures use exponential backoff
and eventually dead-letter; neither path marks capture unhealthy.

Evidence packets redact credential assignments, common API-key forms, and high-entropy tokens
before anything leaves the machine; the ledger stores only redaction category and SHA-256 hash.

## Changing provider

Point `base_url` and `model` somewhere else. If the new endpoint speaks the Responses API,
nothing in the workspace changes. If it does not, `llm_client.rs` is the one file to edit — and
`crates/brain-context/tests/llm_transport.rs` asserts the wire contract against a stub server on
a real socket, so the route, model, auth header, retry and timeout behaviour are all checked
without a billable request.
