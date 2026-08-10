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
    /// Whether a claim in this state may be served to anything that reads the brain.
    ///
    /// **An allowlist, deliberately.** Three read paths carried the same denylist — "not `invalid`
    /// and not `superseded`" — in `current_project_memories`, `current_preferences` and
    /// `resolve_memory_set`. That is fine only while every status is one of three, and it fails
    /// open the moment a fourth appears: A9 began writing `Proposed`, and all three promptly served
    /// memories no human had approved, including into the session-start orientation. The gate would
    /// have been a gate in name only, and nothing about it would have looked wrong.
    ///
    /// This is the same lesson as `CURRENT_CLAIM` — a predicate with a missing half reads as
    /// working code — so the definition lives here once rather than in each caller.
    ///
    /// `Conflict` is readable on purpose. It marks a claim that disagrees with another, not one
    /// that is wrong, and the retrieval layer already weights it below a clean claim rather than
    /// hiding it. Suppressing a contradiction would leave a reader confidently holding one side.
    pub const fn is_readable(&self) -> bool {
        matches!(self, Self::Current | Self::Conflict)
    }

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
            .join(self.projection_file_name())
    }

    /// The note's filename: a readable slug of its title, disambiguated by a short id.
    ///
    /// Obsidian labels every node in its graph view with the **filename**, ignoring both the
    /// `title` front-matter and any `[[id|alias]]` used to link to it. Naming files by memory id
    /// alone produced a graph of several hundred UUIDs — structurally correct and completely
    /// unreadable.
    ///
    /// The id stays, shortened, as a suffix. Titles are neither unique nor path-safe, and two
    /// memories that slug identically would otherwise collide and silently overwrite one
    /// another inside a generation.
    pub fn projection_file_name(&self) -> String {
        let slug = slugify(&self.title);
        let short = self.id.simple().to_string();
        let short = &short[..8.min(short.len())];
        if slug.is_empty() {
            format!("{short}.md")
        } else {
            format!("{slug}-{short}.md")
        }
    }
}

/// Reduce a title to a filename-safe slug.
///
/// Bounded at 80 characters because a projection path already carries a vault root, a project
/// UUID, a generation hash and a date partition, and Windows still enforces a 260-character
/// path limit by default. A title truncated in the middle of a word is a smaller problem than a
/// note that cannot be written at all.
fn slugify(title: &str) -> String {
    let mut slug = String::new();
    let mut pending_dash = false;
    for character in title.chars() {
        if character.is_ascii_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.extend(character.to_lowercase());
            if slug.chars().count() >= 80 {
                break;
            }
        } else {
            pending_dash = true;
        }
    }
    slug.trim_matches('-').to_owned()
}
