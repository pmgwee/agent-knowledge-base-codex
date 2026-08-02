use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_service::{
    ConsolidationCrashPoint, ConsolidationWorker, EvidencePacket, MemoryProposer, WorkerOutcome,
};
use brain_store::{ConsolidationReason, EventLedger};

struct FixtureProposer;

impl MemoryProposer for FixtureProposer {
    fn propose(&self, packet: &EvidencePacket) -> anyhow::Result<Vec<MemoryRecord>> {
        assert!(!packet.serialized().contains("sk-super-secret-token"));
        assert!(!packet.redactions.is_empty());
        let evidence = packet
            .events
            .iter()
            .map(|event| event.event_id)
            .collect::<Vec<_>>();
        Ok(vec![MemoryRecord {
            id: deterministic_id(packet.job_id, 1),
            version_id: deterministic_id(packet.job_id, 2),
            scope: MemoryScope::Project(packet.project_id),
            worktree_id: None,
            task_id: None,
            kind: MemoryKind::Checkpoint,
            title: "Consolidated checkpoint".to_owned(),
            content: "OAuth callback implementation in progress".to_owned(),
            valid_from: time::OffsetDateTime::UNIX_EPOCH,
            valid_to: None,
            recorded_at: time::OffsetDateTime::UNIX_EPOCH,
            confidence: 0.9,
            authority: Authority::DerivedMemory,
            evidence_ids: evidence,
            supersedes: Vec::new(),
            status: MemoryStatus::Current,
        }])
    }
}

#[test]
fn crash_after_memory_write_before_job_ack_does_not_duplicate_memory() {
    let project = ProjectId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let event = append_secret_event(&mut ledger, project);
    let job = ledger
        .enqueue_consolidation_job(event, event, ConsolidationReason::ExplicitCheckpoint)
        .expect("enqueue consolidation");
    let worker = ConsolidationWorker::new("worker-a", time::Duration::seconds(5));
    let started_at = job.available_at;

    let first = worker
        .run_once(
            &mut ledger,
            &FixtureProposer,
            started_at,
            ConsolidationCrashPoint::BeforeJobAck,
        )
        .expect("simulate crash boundary");
    assert_eq!(first, WorkerOutcome::SimulatedCrash(job.id));

    let restarted = ConsolidationWorker::new("worker-b", time::Duration::seconds(5));
    let second = restarted
        .run_once(
            &mut ledger,
            &FixtureProposer,
            started_at + time::Duration::seconds(6),
            ConsolidationCrashPoint::None,
        )
        .expect("replay expired lease");
    assert_eq!(second, WorkerOutcome::Completed(job.id));

    let memory_id = deterministic_id(job.id, 1);
    assert_eq!(
        ledger
            .memory_versions(memory_id)
            .expect("read memory")
            .len(),
        1
    );
    assert!(
        ledger
            .redaction_manifest(job.id)
            .expect("read redactions")
            .iter()
            .any(|entry| entry.category == "api_key")
    );
}

fn append_secret_event(ledger: &mut EventLedger, project: ProjectId) -> uuid::Uuid {
    let event_id = uuid::Uuid::now_v7();
    ledger
        .append_batch(&EventBatch {
            source_id: "fixture".to_owned(),
            events: vec![NormalizedEvent {
                event_id,
                project_id: project,
                worktree_id: WorktreeId(uuid::Uuid::now_v7()),
                task_id: None,
                harness: Harness::ClaudeCode,
                native_session_id: "session".to_owned(),
                native_turn_id: None,
                event_type: EventType::AgentResponded,
                occurred_at: time::OffsetDateTime::UNIX_EPOCH,
                observed_at: time::OffsetDateTime::UNIX_EPOCH,
                source_locator: "fixture".to_owned(),
                source_offset: 1,
                source_schema: "fixture".to_owned(),
                raw_hash: [1; 32],
                idempotency_key: [1; 32],
                git_head: None,
                git_branch: None,
                payload: serde_json::json!({"content": "API_KEY=sk-super-secret-token-1234567890"}),
                raw: serde_json::json!({"content": "API_KEY=sk-super-secret-token-1234567890"}),
            }],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(1),
        })
        .expect("append secret evidence");
    event_id
}

fn deterministic_id(seed: uuid::Uuid, discriminator: u8) -> uuid::Uuid {
    let mut bytes = *seed.as_bytes();
    bytes[15] ^= discriminator;
    uuid::Uuid::from_bytes(bytes)
}
