use std::path::PathBuf;

use anyhow::Result;
use brain_domain::{NormalizedEvent, ProjectId, SourceCursor, WorktreeId};

pub trait SourceAdapter: Send + Sync {
    fn name(&self) -> &'static str;
    fn discover(&self) -> Result<Vec<SourceDescriptor>>;
    fn fingerprint(&self, source: &SourceDescriptor) -> Result<SchemaFingerprint>;
    fn read_increment(
        &self,
        source: &SourceDescriptor,
        cursor: &SourceCursor,
    ) -> Result<ReadOutcome>;
    fn normalize(
        &self,
        record: &RawRecord,
        context: &NormalizeContext,
    ) -> Result<Vec<NormalizedEvent>>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceDescriptor {
    pub source_id: String,
    pub path: PathBuf,
}

impl SourceDescriptor {
    pub fn file(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        Self {
            source_id: format!("file:{}", path.to_string_lossy().to_lowercase()),
            path,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaFingerprint(pub String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileIdentity(pub String);

#[derive(Clone, Debug)]
pub struct RawRecord {
    pub source_id: String,
    pub source_locator: String,
    pub byte_offset: u64,
    pub next_byte_offset: u64,
    pub value: Option<serde_json::Value>,
    pub raw_text: String,
    pub parse_error: Option<String>,
    pub raw_hash: [u8; 32],
}

#[derive(Clone, Debug)]
pub struct RawRecordBatch {
    pub records: Vec<RawRecord>,
    pub next_cursor: SourceCursor,
    pub file_identity: FileIdentity,
    pub last_complete_newline: u64,
    pub rotation: Option<FileRotation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileRotation {
    pub previous_identity: Option<String>,
    pub current_identity: String,
    pub previous_offset: u64,
    pub current_size: u64,
}

#[derive(Clone, Debug)]
pub enum ReadOutcome {
    Batch(RawRecordBatch),
    NoChange,
    SchemaDrift(SchemaDrift),
    SourceUnavailable(SourceUnavailable),
}

#[derive(Clone, Debug)]
pub struct SchemaDrift {
    pub source_id: String,
    pub expected: SchemaFingerprint,
    pub observed: SchemaFingerprint,
    pub sample_hash: [u8; 32],
}

#[derive(Clone, Debug)]
pub struct SourceUnavailable {
    pub source_id: String,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct NormalizeContext {
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub source_schema: String,
}
