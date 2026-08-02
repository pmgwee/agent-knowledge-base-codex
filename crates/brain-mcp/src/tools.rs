use anyhow::{Result, bail};
use brain_service::BrainQueryService;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

const TOOL_NAMES: [&str; 16] = [
    "brain_search",
    "brain_timeline",
    "brain_checkpoint",
    "brain_evidence",
    "brain_correct",
    "brain_status",
    "brain_claim",
    "brain_claims",
    "brain_release_claim",
    "brain_lease_acquire",
    "brain_lease_renew",
    "brain_lease_release",
    "brain_lease_handoff",
    "brain_leases",
    "brain_merge_preflight",
    "brain_context_for_prompt",
];

pub struct BrainTools {
    service: BrainQueryService,
}

impl BrainTools {
    pub const fn new(service: BrainQueryService) -> Self {
        Self { service }
    }

    pub fn has_tool(&self, name: &str) -> bool {
        TOOL_NAMES.contains(&name)
    }

    pub fn definitions(&self) -> Vec<Value> {
        vec![
            tool(
                "brain_search",
                "Search project-scoped canonical events and temporal memories with citations.",
                object_schema(
                    json!({
                        "project": string("Registered project UUID or exact path alias."),
                        "text": string("Terms to search for."),
                        "as_of": timestamp(),
                        "worktree_id": string("Optional worktree UUID."),
                        "task_id": string("Optional task UUID."),
                        "native_session_id": string("Optional native session ID."),
                        "paths": {"type": "array", "items": {"type": "string"}},
                        "source": {"type": "string", "enum": ["all", "events", "memories"]},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 100}
                    }),
                    &["project", "text"],
                ),
                true,
                true,
            ),
            tool(
                "brain_timeline",
                "Return an evidence-cited last-day, last-week, last-month, or custom project timeline.",
                object_schema(
                    json!({
                        "project": string("Registered project UUID or exact path alias."),
                        "window": {"type": "string", "enum": ["day", "week", "month", "custom"], "default": "week"},
                        "start": timestamp(),
                        "end": timestamp(),
                        "now": timestamp(),
                        "as_of": timestamp(),
                        "source": {"type": "string", "enum": ["all", "events", "memories"]},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 100}
                    }),
                    &["project"],
                ),
                true,
                true,
            ),
            tool(
                "brain_checkpoint",
                "Compile the bounded authoritative checkpoint used to orient a new agent session.",
                object_schema(
                    json!({
                        "project": string("Registered project UUID or exact path alias."),
                        "prompt": string("Optional current task question."),
                        "paths": {"type": "array", "items": {"type": "string"}},
                        "as_of": timestamp(),
                        "max_tokens": {"type": "integer", "minimum": 1, "maximum": 3000}
                    }),
                    &["project"],
                ),
                true,
                true,
            ),
            tool(
                "brain_evidence",
                "Fetch one bounded event or memory record by its cited reference.",
                object_schema(
                    json!({
                        "project": string("Registered project UUID or exact path alias."),
                        "reference": string("event:<uuid>, memory:<uuid>, or a bare UUID.")
                    }),
                    &["project", "reference"],
                ),
                true,
                true,
            ),
            tool(
                "brain_correct",
                "Append an audited human-authority memory correction; never overwrites history.",
                object_schema(
                    json!({
                        "project": string("Registered project UUID or exact path alias."),
                        "correction_id": string("Stable UUID used to make retries idempotent."),
                        "memory_id": string("Existing memory UUID, omitted to create one."),
                        "kind": {"type": "string", "enum": ["checkpoint", "decision", "fact", "investigation", "procedure", "deployment", "timeline", "task"]},
                        "title": string("Correction title."),
                        "content": string("Corrected statement."),
                        "evidence_ids": {"type": "array", "items": {"type": "string"}},
                        "valid_from": timestamp()
                    }),
                    &["project", "correction_id", "title", "content"],
                ),
                false,
                true,
            ),
            tool(
                "brain_status",
                "Report canonical project brain health and optional-provider independence.",
                object_schema(
                    json!({"project": string("Registered project UUID or exact path alias.")}),
                    &["project"],
                ),
                true,
                true,
            ),
            tool(
                "brain_claim",
                "Claim repository-relative paths or symbols and return overlap warnings from other active tasks.",
                object_schema(
                    json!({
                        "project": string("Registered project UUID or exact path alias."),
                        "task_id": string("Active coordination task UUID."),
                        "claims": {
                            "type": "array", "minItems": 1, "maxItems": 100,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "kind": {"type": "string", "enum": ["file", "directory", "glob", "symbol"]},
                                    "value": string("Repository-relative path or conservative glob."),
                                    "symbol": string("Required symbol name for symbol claims.")
                                },
                                "required": ["kind", "value"],
                                "additionalProperties": false
                            }
                        }
                    }),
                    &["project", "task_id", "claims"],
                ),
                false,
                false,
            ),
            tool(
                "brain_claims",
                "List active project-scoped path and symbol claims.",
                object_schema(
                    json!({"project": string("Registered project UUID or exact path alias.")}),
                    &["project"],
                ),
                true,
                true,
            ),
            tool(
                "brain_release_claim",
                "Release one path claim owned by a coordination task.",
                object_schema(
                    json!({
                        "project": string("Registered project UUID or exact path alias."),
                        "task_id": string("Owning task UUID."),
                        "claim_id": string("Claim UUID to release.")
                    }),
                    &["project", "task_id", "claim_id"],
                ),
                false,
                true,
            ),
            tool(
                "brain_lease_acquire",
                "Acquire the single writer lease for an active task worktree.",
                object_schema(
                    json!({
                        "project": string("Registered project UUID or exact path alias."),
                        "task_id": string("Active task UUID."),
                        "owner": owner_schema()
                    }),
                    &["project", "task_id", "owner"],
                ),
                false,
                false,
            ),
            tool(
                "brain_lease_renew",
                "Renew a writer lease using its exact owner and generation.",
                lease_generation_schema(),
                false,
                true,
            ),
            tool(
                "brain_lease_release",
                "Release a writer lease using its exact owner and generation.",
                lease_generation_schema(),
                false,
                true,
            ),
            tool(
                "brain_lease_handoff",
                "Write a checkpoint and atomically transfer the writer generation to another session.",
                object_schema(
                    json!({
                        "project": string("Registered project UUID or exact path alias."),
                        "handoff_id": string("Stable UUID for idempotent retries."),
                        "task_id": string("Active task UUID."),
                        "current_owner": owner_schema(),
                        "generation": {"type": "integer", "minimum": 1},
                        "next_owner": owner_schema(),
                        "checkpoint": string("Concise evidence checkpoint for the receiving session.")
                    }),
                    &[
                        "project",
                        "handoff_id",
                        "task_id",
                        "current_owner",
                        "generation",
                        "next_owner",
                        "checkpoint",
                    ],
                ),
                false,
                true,
            ),
            tool(
                "brain_leases",
                "List non-expired writer leases for a project.",
                object_schema(
                    json!({"project": string("Registered project UUID or exact path alias.")}),
                    &["project"],
                ),
                true,
                true,
            ),
            tool(
                "brain_merge_preflight",
                "Predict Git merge conflicts without changing branches, worktrees, or indexes.",
                object_schema(
                    json!({
                        "project": string("Registered project UUID or exact path alias."),
                        "task_id": string("Optional task UUID whose validated worktree is the repository."),
                        "source_ref": string("Source commit/ref; defaults to HEAD."),
                        "target_ref": string("Target commit/ref to test against.")
                    }),
                    &["project", "target_ref"],
                ),
                true,
                true,
            ),
            tool(
                "brain_context_for_prompt",
                "Compile canonical context plus guarded optional knowledge for the first real prompt of a native session.",
                object_schema(
                    json!({
                        "project": string("Registered project UUID or exact path alias."),
                        "harness": {"type": "string", "enum": ["claude-code", "codex", "hermes"]},
                        "native_session_id": string("Stable native session ID used for once-only retrieval."),
                        "prompt": string("The first real user prompt, never the full transcript."),
                        "task_id": string("Optional coordination task UUID for exact worktree scope."),
                        "paths": {"type": "array", "items": {"type": "string"}},
                        "max_tokens": {"type": "integer", "minimum": 1, "maximum": 3000}
                    }),
                    &["project", "harness", "native_session_id", "prompt"],
                ),
                true,
                true,
            ),
        ]
    }

    pub fn call(&self, name: &str, arguments: Value) -> Result<Value> {
        match name {
            "brain_search" => serialize(self.service.search(parse(arguments)?)),
            "brain_timeline" => serialize(self.service.timeline(parse(arguments)?)),
            "brain_checkpoint" => serialize(self.service.checkpoint(parse(arguments)?)),
            "brain_evidence" => serialize(self.service.evidence(parse(arguments)?)),
            "brain_correct" => serialize(self.service.correct(parse(arguments)?)),
            "brain_status" => serialize(self.service.status(parse(arguments)?)),
            "brain_claim" => serialize(self.service.claim(parse(arguments)?)),
            "brain_claims" => serialize(self.service.claims(parse(arguments)?)),
            "brain_release_claim" => serialize(self.service.release_claim(parse(arguments)?)),
            "brain_lease_acquire" => serialize(self.service.acquire_lease(parse(arguments)?)),
            "brain_lease_renew" => serialize(self.service.renew_lease(parse(arguments)?)),
            "brain_lease_release" => serialize(self.service.release_lease(parse(arguments)?)),
            "brain_lease_handoff" => serialize(self.service.handoff_lease(parse(arguments)?)),
            "brain_leases" => serialize(self.service.leases(parse(arguments)?)),
            "brain_merge_preflight" => serialize(self.service.preflight(parse(arguments)?)),
            "brain_context_for_prompt" => {
                serialize(self.service.context_for_prompt(parse(arguments)?))
            }
            _ => bail!("unknown tool {name}"),
        }
    }
}

fn parse<T: DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(Into::into)
}

fn serialize<T: serde::Serialize>(value: Result<T>) -> Result<Value> {
    Ok(serde_json::to_value(value?)?)
}

fn tool(
    name: &str,
    description: &str,
    input_schema: Value,
    read_only: bool,
    idempotent: bool,
) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
        "annotations": {
            "readOnlyHint": read_only,
            "destructiveHint": false,
            "idempotentHint": idempotent,
            "openWorldHint": false
        }
    })
}

fn object_schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn string(description: &str) -> Value {
    json!({"type": "string", "description": description})
}

fn timestamp() -> Value {
    json!({"type": "string", "format": "date-time", "description": "RFC3339 timestamp."})
}

fn owner_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "harness": {"type": "string", "enum": ["claude-code", "codex", "hermes"]},
            "native_session_id": string("Stable native session ID.")
        },
        "required": ["harness", "native_session_id"],
        "additionalProperties": false
    })
}

fn lease_generation_schema() -> Value {
    object_schema(
        json!({
            "project": string("Registered project UUID or exact path alias."),
            "task_id": string("Active task UUID."),
            "owner": owner_schema(),
            "generation": {"type": "integer", "minimum": 1}
        }),
        &["project", "task_id", "owner", "generation"],
    )
}
