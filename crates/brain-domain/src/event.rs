use crate::{ProjectId, WorktreeId};

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Harness {
    ClaudeCode,
    Codex,
    Hermes,
    Other(String),
}

impl Harness {
    pub fn as_str(&self) -> &str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::Hermes => "hermes",
            Self::Other(name) => name,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum EventType {
    #[serde(rename = "session.started")]
    SessionStarted,
    #[serde(rename = "session.resumed")]
    SessionResumed,
    #[serde(rename = "session.compacted")]
    SessionCompacted,
    #[serde(rename = "session.ended")]
    SessionEnded,
    #[serde(rename = "user.prompted")]
    UserPrompted,
    #[serde(rename = "agent.responded")]
    AgentResponded,
    #[serde(rename = "tool.requested")]
    ToolRequested,
    #[serde(rename = "tool.completed")]
    ToolCompleted,
    #[serde(rename = "tool.failed")]
    ToolFailed,
    #[serde(rename = "file.read")]
    FileRead,
    #[serde(rename = "file.created")]
    FileCreated,
    #[serde(rename = "file.modified")]
    FileModified,
    #[serde(rename = "file.deleted")]
    FileDeleted,
    #[serde(rename = "command.started")]
    CommandStarted,
    #[serde(rename = "command.completed")]
    CommandCompleted,
    #[serde(rename = "test.completed")]
    TestCompleted,
    #[serde(rename = "git.commit_observed")]
    GitCommitObserved,
    #[serde(rename = "git.branch_changed")]
    GitBranchChanged,
    #[serde(rename = "deployment.observed")]
    DeploymentObserved,
    #[serde(rename = "task.claimed")]
    TaskClaimed,
    #[serde(rename = "task.released")]
    TaskReleased,
    #[serde(rename = "task.completed")]
    TaskCompleted,
    #[serde(rename = "checkpoint.authored")]
    CheckpointAuthored,
    #[serde(rename = "schema.unknown")]
    SchemaUnknown,
    #[serde(rename = "capture.gap")]
    CaptureGap,
}

impl EventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SessionStarted => "session.started",
            Self::SessionResumed => "session.resumed",
            Self::SessionCompacted => "session.compacted",
            Self::SessionEnded => "session.ended",
            Self::UserPrompted => "user.prompted",
            Self::AgentResponded => "agent.responded",
            Self::ToolRequested => "tool.requested",
            Self::ToolCompleted => "tool.completed",
            Self::ToolFailed => "tool.failed",
            Self::FileRead => "file.read",
            Self::FileCreated => "file.created",
            Self::FileModified => "file.modified",
            Self::FileDeleted => "file.deleted",
            Self::CommandStarted => "command.started",
            Self::CommandCompleted => "command.completed",
            Self::TestCompleted => "test.completed",
            Self::GitCommitObserved => "git.commit_observed",
            Self::GitBranchChanged => "git.branch_changed",
            Self::DeploymentObserved => "deployment.observed",
            Self::TaskClaimed => "task.claimed",
            Self::TaskReleased => "task.released",
            Self::TaskCompleted => "task.completed",
            Self::CheckpointAuthored => "checkpoint.authored",
            Self::SchemaUnknown => "schema.unknown",
            Self::CaptureGap => "capture.gap",
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct NormalizedEvent {
    pub event_id: uuid::Uuid,
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub task_id: Option<uuid::Uuid>,
    pub harness: Harness,
    pub native_session_id: String,
    pub native_turn_id: Option<String>,
    pub event_type: EventType,
    pub occurred_at: time::OffsetDateTime,
    pub observed_at: time::OffsetDateTime,
    pub source_locator: String,
    pub source_offset: i64,
    pub source_schema: String,
    pub raw_hash: [u8; 32],
    pub idempotency_key: [u8; 32],
    pub git_head: Option<String>,
    pub git_branch: Option<String>,
    pub payload: serde_json::Value,
    pub raw: serde_json::Value,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct SourceCursor {
    pub byte_offset: u64,
}

impl SourceCursor {
    pub const fn start() -> Self {
        Self { byte_offset: 0 }
    }

    pub const fn byte_offset(byte_offset: u64) -> Self {
        Self { byte_offset }
    }
}

#[derive(Clone, Debug)]
pub struct EventBatch {
    pub source_id: String,
    pub events: Vec<NormalizedEvent>,
    pub next_cursor: SourceCursor,
}
