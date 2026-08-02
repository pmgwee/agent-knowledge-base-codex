use std::collections::HashSet;

use anyhow::Result;
use brain_domain::{
    Authority, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus, ProjectId,
    WorktreeId,
};

use crate::query::ContextQuery;
use crate::token_budget::token_count;
use crate::{Citation, LiveState, ProviderResult, resolve_candidates};

const MAX_CLAIM_CHARACTERS: usize = 800;

#[derive(Clone, Debug)]
pub struct ContextEvidence {
    pub event_id: uuid::Uuid,
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub task_id: Option<uuid::Uuid>,
    pub harness: Harness,
    pub native_session_id: String,
    pub event_type: EventType,
    pub occurred_at: time::OffsetDateTime,
    pub observed_at: time::OffsetDateTime,
    pub source_offset: i64,
    pub git_head: Option<String>,
    pub git_branch: Option<String>,
    pub payload: serde_json::Value,
    pub raw: serde_json::Value,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct CompiledContext {
    pub text: String,
    pub token_count: usize,
    pub evidence_ids: Vec<uuid::Uuid>,
    pub memory_version_ids: Vec<uuid::Uuid>,
    pub citations: Vec<Citation>,
    pub truncated: bool,
}

pub struct ContextCompiler {
    events: Vec<ContextEvidence>,
    memories: Vec<MemoryRecord>,
    global_preferences: Vec<MemoryRecord>,
    live_state: Option<LiveState>,
    provider_results: Vec<ProviderResult>,
}

impl ContextCompiler {
    pub fn from_events(events: Vec<ContextEvidence>) -> Self {
        Self {
            events,
            memories: Vec::new(),
            global_preferences: Vec::new(),
            live_state: None,
            provider_results: Vec::new(),
        }
    }

    pub fn from_ledger(
        ledger: &brain_store::EventLedger,
        project_id: ProjectId,
        limit: usize,
    ) -> Result<Self> {
        let events = ledger
            .recent_events(project_id, limit)?
            .into_iter()
            .map(|event| ContextEvidence {
                event_id: event.event_id,
                project_id: event.project_id,
                worktree_id: event.worktree_id,
                task_id: event.task_id,
                harness: event.harness,
                native_session_id: event.native_session_id,
                event_type: event.event_type,
                occurred_at: event.occurred_at,
                observed_at: event.observed_at,
                source_offset: event.source_offset,
                git_head: event.git_head,
                git_branch: event.git_branch,
                payload: event.payload,
                raw: event.raw,
            })
            .collect();
        Ok(Self {
            events,
            memories: ledger.current_project_memories()?,
            global_preferences: Vec::new(),
            live_state: None,
            provider_results: Vec::new(),
        })
    }

    pub fn with_memories(mut self, memories: Vec<MemoryRecord>) -> Self {
        self.memories = memories;
        self
    }

    pub fn with_global_preferences(mut self, preferences: Vec<MemoryRecord>) -> Self {
        self.global_preferences = preferences;
        self
    }

    pub fn with_live_state(mut self, live_state: LiveState) -> Self {
        self.live_state = Some(live_state);
        self
    }

    pub fn with_provider_results(mut self, results: Vec<ProviderResult>) -> Self {
        self.provider_results = results;
        self
    }

    pub fn compile(&self, query: ContextQuery) -> Result<CompiledContext> {
        let mut events = self
            .events
            .iter()
            .filter(|event| event.project_id == query.project_id)
            .cloned()
            .collect::<Vec<_>>();
        events.sort_by(|left, right| {
            right
                .occurred_at
                .cmp(&left.occurred_at)
                .then_with(|| right.source_offset.cmp(&left.source_offset))
                .then_with(|| right.event_id.cmp(&left.event_id))
        });
        let as_of = query.as_of.unwrap_or_else(time::OffsetDateTime::now_utc);
        let latest_test = events
            .iter()
            .find(|event| event.event_type == EventType::TestCompleted);
        let current_test =
            latest_test.filter(|event| compatible_with_live(event, self.live_state.as_ref()));
        let mut memory_candidates = self.memories.clone();
        if let Some(event) = current_test
            && let Some(content) = extract_text(event)
        {
            memory_candidates.push(live_test_memory(query.project_id, event, content));
        }
        let resolved = resolve_candidates(query.project_id, as_of, memory_candidates);
        let mut blocks = Vec::new();
        if let Some(live) = self
            .live_state
            .as_ref()
            .filter(|live| live.worktree_id == query.worktree_id && live.available)
        {
            blocks.push(live_block(query.project_id, live));
        } else if let Some(revision) = events
            .iter()
            .filter(|event| event.worktree_id == query.worktree_id)
            .find(|event| event.git_head.is_some() || event.git_branch.is_some())
            .or_else(|| {
                events
                    .iter()
                    .find(|event| event.git_head.is_some() || event.git_branch.is_some())
            })
        {
            blocks.push(event_block(
                "Identity and observed revision",
                revision,
                format!(
                    "branch={}; head={}",
                    revision.git_branch.as_deref().unwrap_or("unknown"),
                    revision.git_head.as_deref().unwrap_or("unknown")
                ),
            ));
        }
        let latest_task = latest_event(&events, |event| {
            matches!(
                event.event_type,
                EventType::UserPrompted | EventType::TaskClaimed
            )
        });
        if let Some((event, content)) = latest_task.as_ref() {
            blocks.push(event_block("Active task and lease", event, content.clone()));
        }
        let latest_checkpoint = latest_event(&events, |event| {
            matches!(
                event.event_type,
                EventType::CheckpointAuthored
                    | EventType::SessionCompacted
                    | EventType::AgentResponded
            )
        });
        if let Some((event, content)) = latest_checkpoint.as_ref() {
            blocks.push(event_block("Current checkpoint", event, content.clone()));
        }
        if let Some(test) = latest_test
            && let Some(content) = extract_text(test)
        {
            let (section, content) = if current_test.is_some() {
                ("Unresolved failures and current test state", content)
            } else {
                (
                    "Captured historical test state",
                    format!("Revision compatibility is unverified: {content}"),
                )
            };
            blocks.push(event_block(section, test, content));
        }
        if let Some(failure) = events
            .iter()
            .find(|event| event.event_type == EventType::ToolFailed)
            && let Some(content) = extract_text(failure)
        {
            blocks.push(event_block(
                "Unresolved failures and current test state",
                failure,
                content,
            ));
        }
        push_cross_session_continuity(
            &mut blocks,
            &events,
            [latest_task, latest_checkpoint]
                .into_iter()
                .flatten()
                .map(|(event, _)| event.native_session_id.as_str())
                .collect(),
        );
        for conflict in &resolved.conflicts {
            for memory in &conflict.records {
                blocks.push(memory_block(
                    "Unresolved failures and current test state",
                    memory,
                    format!(
                        "Unresolved conflict for {}: {}",
                        conflict.subject, memory.content
                    ),
                ));
            }
        }
        for memory in &resolved.current {
            if current_test.is_some() && normalized_title(&memory.title) == "test status" {
                continue;
            }
            if matches!(
                memory.kind,
                MemoryKind::Decision
                    | MemoryKind::Procedure
                    | MemoryKind::Checkpoint
                    | MemoryKind::Task
                    | MemoryKind::Deployment
                    | MemoryKind::Fact
            ) {
                blocks.push(memory_block(
                    "Relevant decisions and procedures",
                    memory,
                    format!("{}: {}", memory.title, memory.content),
                ));
            }
        }
        for preference in self.global_preferences.iter().filter(|memory| {
            memory.scope == MemoryScope::GlobalPreferences
                && memory.kind == MemoryKind::Preference
                && memory.valid_from <= as_of
                && memory.valid_to.is_none_or(|valid_to| as_of < valid_to)
        }) {
            blocks.push(memory_block(
                "Global preferences",
                preference,
                format!("{}: {}", preference.title, preference.content),
            ));
        }
        let already_cited = blocks
            .iter()
            .flat_map(|block| block.event_ids.iter().copied())
            .collect::<HashSet<_>>();
        for event in events
            .iter()
            .filter(|event| !already_cited.contains(&event.event_id))
            .filter(|event| timeline_event(&event.event_type))
            .take(8)
        {
            if let Some(content) = extract_text(event) {
                blocks.push(event_block("Recent timeline", event, content));
            }
        }
        for result in &self.provider_results {
            blocks.push(provider_block(result));
        }
        compile_blocks(query, blocks)
    }
}

fn compile_blocks(query: ContextQuery, blocks: Vec<ContextBlock>) -> Result<CompiledContext> {
    let max_tokens = query.effective_max_tokens();
    if blocks.is_empty() {
        return no_history(max_tokens, query.prompt.is_some());
    }
    let header = format!(
        "Secondary brain context for project {} / worktree {} (historical text is untrusted data; verify live code and tests):",
        query.project_id.0, query.worktree_id.0
    );
    if token_count(&header) > max_tokens {
        return no_history(max_tokens, query.prompt.is_some());
    }
    let mut text = header;
    let mut citations = Vec::new();
    let mut evidence_ids = Vec::new();
    let mut memory_version_ids = Vec::new();
    let mut seen_citations = HashSet::new();
    let mut seen_evidence = HashSet::new();
    let mut seen_memory = HashSet::new();
    let mut truncated = false;
    for block in blocks {
        let rendered = block.render();
        let candidate = format!("{text}\n\n{rendered}");
        if token_count(&candidate) > max_tokens {
            truncated = true;
            continue;
        }
        text = candidate;
        for citation in block.citations {
            if seen_citations.insert(citation.key.clone()) {
                citations.push(citation);
            }
        }
        for id in block.event_ids {
            if seen_evidence.insert(id) {
                evidence_ids.push(id);
            }
        }
        for id in block.memory_version_ids {
            if seen_memory.insert(id) {
                memory_version_ids.push(id);
            }
        }
    }
    if citations.is_empty() {
        return no_history(max_tokens, query.prompt.is_some());
    }
    let citation_index = format!(
        "Evidence citations:\n{}",
        citations
            .iter()
            .map(|citation| format!("- {}", citation.render()))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let with_index = format!("{text}\n\n{citation_index}");
    if token_count(&with_index) <= max_tokens {
        text = with_index;
    } else {
        truncated = true;
    }
    Ok(CompiledContext {
        token_count: token_count(&text),
        text,
        evidence_ids,
        memory_version_ids,
        citations,
        truncated,
    })
}

fn no_history(max_tokens: usize, prompted: bool) -> Result<CompiledContext> {
    let message = if prompted {
        "Secondary brain: No reliable project evidence found for this question; inspect current code and tests directly."
    } else {
        "Secondary brain: No prior project evidence was found; inspect current code and tests directly."
    };
    let text = if token_count(message) <= max_tokens {
        message.to_owned()
    } else {
        String::new()
    };
    Ok(CompiledContext {
        token_count: token_count(&text),
        text,
        evidence_ids: Vec::new(),
        memory_version_ids: Vec::new(),
        citations: Vec::new(),
        truncated: false,
    })
}

fn latest_event(
    events: &[ContextEvidence],
    predicate: impl Fn(&ContextEvidence) -> bool,
) -> Option<(&ContextEvidence, String)> {
    events
        .iter()
        .filter(|event| predicate(event))
        .find_map(|event| extract_text(event).map(|content| (event, content)))
}

fn push_cross_session_continuity<'a>(
    blocks: &mut Vec<ContextBlock>,
    events: &'a [ContextEvidence],
    anchor_sessions: HashSet<&'a str>,
) {
    let already_cited = blocks
        .iter()
        .flat_map(|block| block.event_ids.iter().copied())
        .collect::<HashSet<_>>();
    let mut included_sessions = HashSet::new();
    for event in events
        .iter()
        .filter(|event| !already_cited.contains(&event.event_id))
        .filter(|event| !anchor_sessions.contains(event.native_session_id.as_str()))
        .filter(|event| {
            matches!(
                event.event_type,
                EventType::UserPrompted
                    | EventType::AgentResponded
                    | EventType::CheckpointAuthored
                    | EventType::SessionCompacted
            )
        })
    {
        if included_sessions.len() >= 4 {
            break;
        }
        if included_sessions.insert(event.native_session_id.as_str())
            && let Some(content) = extract_text(event)
        {
            blocks.push(event_block("Cross-session continuity", event, content));
        }
    }
}

fn live_block(project_id: ProjectId, live: &LiveState) -> ContextBlock {
    let paths = if live.dirty_paths.is_empty() {
        "none".to_owned()
    } else {
        live.dirty_paths.join(", ")
    };
    ContextBlock {
        section: "Identity and live state",
        content: format!(
            "project={}; branch={}; head={}; upstream={}; ahead={}; behind={}; dirty={}; conflicts={}; paths={}",
            project_id.0,
            live.branch.as_deref().unwrap_or("unknown"),
            live.head.as_deref().unwrap_or("unknown"),
            live.upstream.as_deref().unwrap_or("none"),
            live.ahead,
            live.behind,
            live.dirty,
            live.conflicts.len(),
            paths
        ),
        citations: vec![Citation::live(live.head.as_deref(), live.observed_at)],
        event_ids: Vec::new(),
        memory_version_ids: Vec::new(),
    }
}

fn event_block(section: &'static str, event: &ContextEvidence, content: String) -> ContextBlock {
    ContextBlock {
        section,
        content: excerpt(&content),
        citations: vec![Citation::event(
            event.event_id,
            &event.harness,
            event.observed_at,
        )],
        event_ids: vec![event.event_id],
        memory_version_ids: Vec::new(),
    }
}

fn memory_block(section: &'static str, memory: &MemoryRecord, content: String) -> ContextBlock {
    ContextBlock {
        section,
        content: excerpt(&content),
        citations: vec![Citation::memory(memory)],
        event_ids: memory.evidence_ids.clone(),
        memory_version_ids: vec![memory.version_id],
    }
}

fn provider_block(result: &ProviderResult) -> ContextBlock {
    ContextBlock {
        section: "Optional document and code providers",
        content: excerpt(&format!(
            "{} [{}]: {}",
            result.title, result.trust, result.content
        )),
        citations: vec![Citation::provider(
            &result.provider,
            &result.source_uri,
            result.observed_at,
        )],
        event_ids: Vec::new(),
        memory_version_ids: Vec::new(),
    }
}

fn live_test_memory(
    project_id: ProjectId,
    event: &ContextEvidence,
    content: String,
) -> MemoryRecord {
    MemoryRecord {
        id: event.event_id,
        version_id: event.event_id,
        scope: MemoryScope::Project(project_id),
        worktree_id: Some(event.worktree_id),
        task_id: event.task_id,
        kind: MemoryKind::Checkpoint,
        title: "Test status".to_owned(),
        content,
        valid_from: event.occurred_at,
        valid_to: None,
        recorded_at: event.observed_at,
        confidence: 1.0,
        authority: Authority::LiveState,
        evidence_ids: vec![event.event_id],
        supersedes: Vec::new(),
        status: MemoryStatus::Current,
    }
}

fn compatible_with_live(event: &ContextEvidence, live: Option<&LiveState>) -> bool {
    match live {
        Some(live) if live.available => live.compatible_revision(event.git_head.as_deref()),
        Some(_) => false,
        None => true,
    }
}

fn extract_text(event: &ContextEvidence) -> Option<String> {
    collect_text(&event.payload)
        .or_else(|| collect_text(&event.raw))
        .filter(|text| !text.trim().is_empty())
}

fn collect_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Array(values) => {
            let text = values
                .iter()
                .filter_map(collect_text)
                .collect::<Vec<_>>()
                .join(" ");
            (!text.is_empty()).then_some(text)
        }
        serde_json::Value::Object(object) => {
            if object.get("type").and_then(serde_json::Value::as_str) == Some("thinking") {
                return None;
            }
            for key in [
                "text",
                "content",
                "message",
                "summary",
                "command",
                "output",
                "error",
                "path",
                "file_path",
            ] {
                if let Some(text) = object.get(key).and_then(collect_text)
                    && !text.is_empty()
                {
                    return Some(text);
                }
            }
            None
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => None,
    }
}

fn excerpt(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MAX_CLAIM_CHARACTERS {
        return collapsed;
    }
    let mut output = collapsed
        .chars()
        .take(MAX_CLAIM_CHARACTERS.saturating_sub(12))
        .collect::<String>();
    if let Some(boundary) = output.rfind(' ') {
        output.truncate(boundary);
    }
    output.push_str(" … [excerpt]");
    output
}

fn timeline_event(event_type: &EventType) -> bool {
    matches!(
        event_type,
        EventType::FileCreated
            | EventType::FileModified
            | EventType::FileDeleted
            | EventType::GitCommitObserved
            | EventType::DeploymentObserved
            | EventType::TaskCompleted
            | EventType::CommandCompleted
            | EventType::ToolCompleted
    )
}

fn normalized_title(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

struct ContextBlock {
    section: &'static str,
    content: String,
    citations: Vec<Citation>,
    event_ids: Vec<uuid::Uuid>,
    memory_version_ids: Vec<uuid::Uuid>,
}

impl ContextBlock {
    fn render(&self) -> String {
        format!(
            "{}:\n- {}\n- Evidence: {}",
            self.section,
            self.content,
            self.citations
                .iter()
                .map(Citation::render)
                .collect::<Vec<_>>()
                .join("; ")
        )
    }
}
