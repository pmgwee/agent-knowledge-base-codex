use std::path::PathBuf;

use crate::{ProjectId, WorktreeId};

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Checkpoint,
    Decision,
    Fact,
    Investigation,
    Procedure,
    Deployment,
    Timeline,
    Preference,
    Task,
}

impl MemoryKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Checkpoint => "checkpoint",
            Self::Decision => "decision",
            Self::Fact => "fact",
            Self::Investigation => "investigation",
            Self::Procedure => "procedure",
            Self::Deployment => "deployment",
            Self::Timeline => "timeline",
            Self::Preference => "preference",
            Self::Task => "task",
        }
    }

    pub fn from_name(value: &str) -> Option<Self> {
        Some(match value {
            "checkpoint" => Self::Checkpoint,
            "decision" => Self::Decision,
            "fact" => Self::Fact,
            "investigation" => Self::Investigation,
            "procedure" => Self::Procedure,
            "deployment" => Self::Deployment,
            "timeline" => Self::Timeline,
            "preference" => Self::Preference,
            "task" => Self::Task,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "scope", content = "project_id", rename_all = "snake_case")]
pub enum MemoryScope {
    Project(ProjectId),
    GlobalPreferences,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Authority {
    ExternalDocument,
    DerivedMemory,
    AgentCheckpoint,
    HumanCorrection,
    RawMechanicalEvidence,
    LiveState,
}

impl Authority {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::ExternalDocument => "external_document",
            Self::DerivedMemory => "derived_memory",
            Self::AgentCheckpoint => "agent_checkpoint",
            Self::HumanCorrection => "human_correction",
            Self::RawMechanicalEvidence => "raw_mechanical_evidence",
            Self::LiveState => "live_state",
        }
    }

    pub fn from_name(value: &str) -> Option<Self> {
        Some(match value {
            "external_document" => Self::ExternalDocument,
            "derived_memory" => Self::DerivedMemory,
            "agent_checkpoint" => Self::AgentCheckpoint,
            "human_correction" => Self::HumanCorrection,
            "raw_mechanical_evidence" => Self::RawMechanicalEvidence,
            "live_state" => Self::LiveState,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Proposed,
    Current,
    Superseded,
    Conflict,
    Invalid,
}

impl MemoryStatus {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Current => "current",
            Self::Superseded => "superseded",
            Self::Conflict => "conflict",
            Self::Invalid => "invalid",
        }
    }

    pub fn from_name(value: &str) -> Option<Self> {
        Some(match value {
            "proposed" => Self::Proposed,
            "current" => Self::Current,
            "superseded" => Self::Superseded,
            "conflict" => Self::Conflict,
            "invalid" => Self::Invalid,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct MemoryRecord {
    pub id: uuid::Uuid,
    pub version_id: uuid::Uuid,
    pub scope: MemoryScope,
    pub worktree_id: Option<WorktreeId>,
    pub task_id: Option<uuid::Uuid>,
    pub kind: MemoryKind,
    pub title: String,
    pub content: String,
    pub valid_from: time::OffsetDateTime,
    pub valid_to: Option<time::OffsetDateTime>,
    pub recorded_at: time::OffsetDateTime,
    pub confidence: f32,
    pub authority: Authority,
    pub evidence_ids: Vec<uuid::Uuid>,
    pub supersedes: Vec<uuid::Uuid>,
    pub status: MemoryStatus,
}

impl MemoryRecord {
    pub fn projection_path(&self) -> PathBuf {
        let partition_time = self
            .id
            .get_timestamp()
            .and_then(|timestamp| {
                let (seconds, nanoseconds) = timestamp.to_unix();
                time::OffsetDateTime::from_unix_timestamp(i64::try_from(seconds).ok()?)
                    .ok()?
                    .replace_nanosecond(nanoseconds)
                    .ok()
            })
            .unwrap_or(self.valid_from);
        let root = match self.scope {
            MemoryScope::Project(project_id) => {
                PathBuf::from("projects").join(project_id.0.to_string())
            }
            MemoryScope::GlobalPreferences => PathBuf::from("global-preferences"),
        };
        root.join("generated")
            .join(self.kind.as_str())
            .join(format!("{:04}", partition_time.year()))
            .join(format!("{:02}", u8::from(partition_time.month())))
            .join(format!("{}.md", self.id))
    }
}
