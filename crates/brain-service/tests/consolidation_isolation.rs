use std::sync::Mutex;

use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_service::{ConsolidationCrashPoint, ConsolidationWorker, EvidencePacket, MemoryProposer};
use brain_store::{ConsolidationReason, EventLedger};

#[derive(Default)]
struct RecordingProposer {
    projects: Mutex<Vec<ProjectId>>,
}

impl MemoryProposer for RecordingProposer {
    fn propose(&self, packet: &EvidencePacket) -> anyhow::Result<Vec<MemoryRecord>> {
        self.projects
            .lock()
            .expect("lock projects")
            .push(packet.project_id);
        let content = packet.serialized();
        let mut id_bytes = *packet.job_id.as_bytes();
        id_bytes[15] ^= 1;
        let memory_id = uuid::Uuid::from_bytes(id_bytes);
        id_bytes[15] ^= 3;
        Ok(vec![MemoryRecord {
            id: memory_id,
            version_id: uuid::Uuid::from_bytes(id_bytes),
            scope: MemoryScope::Project(packet.project_id),
            worktree_id: None,
            task_id: None,
            kind: MemoryKind::Timeline,
            title: "Project timeline".to_owned(),
            content,
            valid_from: time::OffsetDateTime::UNIX_EPOCH,
            valid_to: None,
            recorded_at: time::OffsetDateTime::UNIX_EPOCH,
            confidence: 1.0,
            authority: Authority::DerivedMemory,
            evidence_ids: packet.events.iter().map(|event| event.event_id).collect(),
            supersedes: Vec::new(),
            status: MemoryStatus::Current,
        }])
    }
}

#[test]
fn consolidation_packets_and_memories_never_mix_projects() {
    let project_a = ProjectId(uuid::Uuid::now_v7());
    let project_b = ProjectId(uuid::Uuid::now_v7());
    let mut ledger_a = EventLedger::open_in_memory(project_a).expect("open A");
    let mut ledger_b = EventLedger::open_in_memory(project_b).expect("open B");
    let event_a = append(&mut ledger_a, project_a, "PROJECT_A_ONLY");
    let event_b = append(&mut ledger_b, project_b, "PROJECT_B_ONLY");
    let job_a = ledger_a
        .enqueue_consolidation_job(event_a, event_a, ConsolidationReason::ExplicitCheckpoint)
        .expect("enqueue A");
    let job_b = ledger_b
        .enqueue_consolidation_job(event_b, event_b, ConsolidationReason::ExplicitCheckpoint)
        .expect("enqueue B");
    let proposer = RecordingProposer::default();
    let worker = ConsolidationWorker::new("worker", time::Duration::seconds(30));
    worker
        .run_once(
            &mut ledger_a,
            &proposer,
            job_a.available_at,
            ConsolidationCrashPoint::None,
        )
        .expect("consolidate A");
    worker
        .run_once(
            &mut ledger_b,
            &proposer,
            job_b.available_at,
            ConsolidationCrashPoint::None,
        )
        .expect("consolidate B");

    assert_eq!(
        *proposer.projects.lock().expect("read projects"),
        vec![project_a, project_b]
    );
    assert!(!ledger_a.raw_contains("PROJECT_B_ONLY").expect("scan A raw"));
    assert!(!ledger_b.raw_contains("PROJECT_A_ONLY").expect("scan B raw"));
}

fn append(ledger: &mut EventLedger, project: ProjectId, sentinel: &str) -> uuid::Uuid {
    let event_id = uuid::Uuid::now_v7();
    ledger
        .append_batch(&EventBatch {
            source_id: "fixture".to_owned(),
            events: vec![NormalizedEvent {
                event_id,
                project_id: project,
                worktree_id: WorktreeId(uuid::Uuid::now_v7()),
                task_id: None,
                harness: Harness::Codex,
                native_session_id: "session".to_owned(),
                native_turn_id: None,
                event_type: EventType::CheckpointAuthored,
                occurred_at: time::OffsetDateTime::UNIX_EPOCH,
                observed_at: time::OffsetDateTime::UNIX_EPOCH,
                source_locator: "fixture".to_owned(),
                source_offset: 1,
                source_schema: "fixture".to_owned(),
                raw_hash: [1; 32],
                idempotency_key: [1; 32],
                git_head: None,
                git_branch: None,
                payload: serde_json::json!({"content": sentinel}),
                raw: serde_json::json!({"content": sentinel}),
            }],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(1),
        })
        .expect("append event");
    event_id
}
