use brain_domain::{
    EventBatch, EventType, Harness, MemoryKind, NormalizedEvent, ProjectId, SourceCursor,
    WorktreeId,
};
use brain_service::{
    ConsolidationCrashPoint, ConsolidationLlm, ConsolidationWorker, EvidencePacket, ProposedMemory,
    ProposedMemoryBatch, WorkerOutcome,
};
use brain_store::{ConsolidationReason, EventLedger, JobStatus};

struct FixtureProposer;

#[async_trait::async_trait]
impl ConsolidationLlm for FixtureProposer {
    async fn propose(&self, packet: &EvidencePacket) -> anyhow::Result<ProposedMemoryBatch> {
        assert!(!packet.serialized().contains("sk-super-secret-token"));
        assert!(!packet.redactions.is_empty());
        Ok(ProposedMemoryBatch {
            memories: vec![ProposedMemory {
                kind: MemoryKind::Checkpoint,
                title: "Consolidated checkpoint".to_owned(),
                content: "OAuth callback implementation in progress".to_owned(),
                valid_from: time::OffsetDateTime::UNIX_EPOCH,
                confidence: 0.9,
                evidence_ids: packet.events.iter().map(|event| event.event_id).collect(),
                supersedes: Vec::new(),
            }],
        })
    }
}

struct UnavailableProposer(&'static str);

#[async_trait::async_trait]
impl ConsolidationLlm for UnavailableProposer {
    async fn propose(&self, _packet: &EvidencePacket) -> anyhow::Result<ProposedMemoryBatch> {
        anyhow::bail!(self.0)
    }
}

#[tokio::test]
async fn crash_after_memory_write_before_job_ack_does_not_duplicate_memory() {
    let project = ProjectId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let event = append_secret_event(&mut ledger, project, [1; 32]);
    let job = ledger
        .enqueue_consolidation_job(event, event, ConsolidationReason::ExplicitCheckpoint)
        .expect("enqueue consolidation");
    let worker = ConsolidationWorker::new("worker-a", time::Duration::seconds(5));

    let first = worker
        .run_once(
            &mut ledger,
            &FixtureProposer,
            job.available_at,
            ConsolidationCrashPoint::BeforeJobAck,
        )
        .await
        .expect("simulate crash boundary");
    assert_eq!(first, WorkerOutcome::SimulatedCrash(job.id));

    let second = ConsolidationWorker::new("worker-b", time::Duration::seconds(5))
        .run_once(
            &mut ledger,
            &FixtureProposer,
            job.available_at + time::Duration::seconds(6),
            ConsolidationCrashPoint::None,
        )
        .await
        .expect("replay expired lease");
    assert_eq!(second, WorkerOutcome::Completed(job.id));
    assert_eq!(ledger.memory_count().expect("count memory"), 1);
    assert!(
        ledger
            .redaction_manifest(job.id)
            .expect("read redactions")
            .iter()
            .any(|entry| entry.category == "api_key")
    );
}

#[tokio::test]
async fn unavailable_provider_leaves_the_job_retryable() {
    for (offset, failure) in [
        (
            2,
            "LLM request failed with HTTP status 500 Internal Server Error",
        ),
        (3, "LLM request failed with HTTP status 401 Unauthorized"),
        (
            4,
            "LLM API key environment variable LLM_API_KEY is unavailable",
        ),
    ] {
        let project = ProjectId(uuid::Uuid::now_v7());
        let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
        let event = append_secret_event(&mut ledger, project, [offset; 32]);
        let job = ledger
            .enqueue_consolidation_job(event, event, ConsolidationReason::Inactivity)
            .expect("enqueue consolidation");
        let outcome = ConsolidationWorker::new("worker", time::Duration::seconds(5))
            .run_once(
                &mut ledger,
                &UnavailableProposer(failure),
                job.available_at,
                ConsolidationCrashPoint::None,
            )
            .await
            .expect("provider outage is contained");
        assert_eq!(outcome, WorkerOutcome::ProviderUnavailable(job.id));
        let deferred = ledger
            .consolidation_job(job.id)
            .expect("read job")
            .expect("job exists");
        assert_eq!(deferred.status, JobStatus::Pending);
        assert_eq!(deferred.attempt, 0, "failure must not age job: {failure}");
        assert_eq!(ledger.memory_count().expect("count memory"), 0);
        assert_eq!(ledger.event_count().expect("raw evidence remains"), 1);
    }
}

fn append_secret_event(
    ledger: &mut EventLedger,
    project: ProjectId,
    idempotency_key: [u8; 32],
) -> uuid::Uuid {
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
                raw_hash: idempotency_key,
                idempotency_key,
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
