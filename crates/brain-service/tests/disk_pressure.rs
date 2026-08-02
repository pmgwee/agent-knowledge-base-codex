use std::sync::Arc;

use anyhow::Result;
use brain_adapters::{ClaudeAdapter, NormalizeContext, SourceDescriptor};
use brain_domain::{ProjectId, WorktreeId};
use brain_service::{
    CaptureBinding, CaptureServiceConfig, CaptureSupervisor, DiskProbe, DiskSample,
    PressureController, PressurePolicy,
};

#[test]
fn pressure_pauses_derived_work_before_capture_and_recovers_with_hysteresis() {
    let policy = policy();
    let mut controller = PressureController::new(policy).expect("controller");
    let critical = controller.evaluate(DiskSample {
        available_bytes: 150,
        total_bytes: 10_000,
    });
    assert!(critical.provider_refresh_paused);
    assert!(critical.basic_memory_paused);
    assert!(critical.markdown_projection_paused);
    assert!(critical.consolidation_paused);
    assert!(critical.cold_cache_paused);
    assert!(!critical.capture_blocked);

    let first_recovery = controller.evaluate(DiskSample {
        available_bytes: 900,
        total_bytes: 1_000,
    });
    assert!(first_recovery.consolidation_paused);
    let recovered = controller.evaluate(DiskSample {
        available_bytes: 900,
        total_bytes: 1_000,
    });
    assert_eq!(recovered, Default::default());
}

#[tokio::test]
async fn full_disk_blocks_capture_without_advancing_the_cursor() {
    let temp = tempfile::tempdir().expect("temp");
    let transcript = temp.path().join("session.jsonl");
    std::fs::write(
        &transcript,
        "{\"type\":\"future-event\",\"sessionId\":\"pressure\"}\n",
    )
    .expect("source");
    let project = ProjectId(uuid::Uuid::now_v7());
    let ledger_path = temp.path().join("brain/project/ledger.sqlite");
    let source = SourceDescriptor::file(&transcript);
    let supervisor = CaptureSupervisor::with_config_and_disk_probe(
        vec![CaptureBinding::new(
            Arc::new(ClaudeAdapter::new(temp.path())),
            source.clone(),
            NormalizeContext {
                project_id: project,
                worktree_id: WorktreeId(uuid::Uuid::now_v7()),
                source_schema: "fixture".to_owned(),
            },
            &ledger_path,
        )],
        CaptureServiceConfig::default(),
        policy(),
        Arc::new(FixedProbe(DiskSample {
            available_bytes: 0,
            total_bytes: 1_000,
        })),
    )
    .expect("supervisor");
    supervisor
        .capture_once()
        .await
        .expect("blocked capture is visible, not fatal");

    let health = supervisor.health().expect("health");
    assert!(health.operations.degradation.capture_blocked);
    assert_eq!(
        health
            .sources
            .values()
            .next()
            .expect("source")
            .last_cursor
            .byte_offset,
        0
    );
    assert_eq!(
        brain_store::EventLedger::open(&ledger_path, project)
            .expect("ledger")
            .event_count()
            .expect("count"),
        0
    );
}

fn policy() -> PressurePolicy {
    PressurePolicy {
        warning_free_bytes: 300,
        critical_free_bytes: 200,
        emergency_free_bytes: 50,
        recovery_free_bytes: 800,
        warning_free_percent: 30.0,
        critical_free_percent: 20.0,
        recovery_free_percent: 80.0,
        recovery_checks: 2,
    }
}

struct FixedProbe(DiskSample);

impl DiskProbe for FixedProbe {
    fn sample(&self, _path: &std::path::Path) -> Result<DiskSample> {
        Ok(self.0)
    }
}
