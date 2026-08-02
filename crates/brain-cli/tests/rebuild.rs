use std::sync::Mutex;

use anyhow::Result;
use brain_cli::{rebuild_basic_memory_with, rebuild_markdown, verify_projections};
use brain_domain::{ProjectId, WorktreeId};
use brain_service::{ServiceLaunchConfig, ServiceProjectConfig};
use brain_store::{BASIC_MEMORY_PINNED_VERSION, BasicMemoryCli, BasicMemoryState, EventLedger};

#[test]
fn rebuild_and_verify_use_only_canonical_project_state() {
    let fixture = Fixture::new();
    let markdown =
        rebuild_markdown(&fixture.brain_home, fixture.project).expect("rebuild Markdown");
    assert_eq!(markdown.file_count, 0);
    assert!(
        verify_projections(&fixture.brain_home, fixture.project)
            .expect("verify Markdown")
            .valid
    );

    let cli = FixtureBasicMemory::default();
    let basic = rebuild_basic_memory_with(&fixture.brain_home, fixture.project, &cli)
        .expect("rebuild Basic Memory");
    assert_eq!(basic.state, BasicMemoryState::Ready);
    assert_eq!(cli.calls.lock().expect("lock calls").len(), 2);
    assert!(fixture.ledger_path.is_file());
}

struct Fixture {
    _temp: tempfile::TempDir,
    brain_home: std::path::PathBuf,
    project: ProjectId,
    ledger_path: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("create rebuild fixture");
        let brain_home = temp.path().join("brain");
        let project = ProjectId(uuid::Uuid::now_v7());
        let ledger_path = brain_home
            .join("projects")
            .join(project.0.to_string())
            .join("events.sqlite");
        EventLedger::open(&ledger_path, project).expect("create ledger");
        let mut config = ServiceLaunchConfig::new(r"\\.\pipe\fixture");
        config.upsert_project(ServiceProjectConfig {
            project_root: temp.path().join("project"),
            project_id: project,
            worktree_id: WorktreeId(uuid::Uuid::now_v7()),
            ledger_path: ledger_path.clone(),
            claude_sources: Vec::new(),
            codex_sources: Vec::new(),
            hermes_database: None,
        });
        let config_path = ServiceLaunchConfig::default_path(&brain_home);
        std::fs::create_dir_all(config_path.parent().expect("config parent"))
            .expect("create config root");
        std::fs::write(
            &config_path,
            serde_json::to_vec_pretty(&config).expect("serialize config"),
        )
        .expect("write config");
        Self {
            _temp: temp,
            brain_home,
            project,
            ledger_path,
        }
    }
}

#[derive(Default)]
struct FixtureBasicMemory {
    calls: Mutex<Vec<String>>,
}

impl BasicMemoryCli for FixtureBasicMemory {
    fn version(&self) -> Result<String> {
        Ok(format!("basic-memory {BASIC_MEMORY_PINNED_VERSION}"))
    }

    fn ensure_project(&self, name: &str, _root: &std::path::Path) -> Result<()> {
        self.calls
            .lock()
            .expect("lock calls")
            .push(format!("add:{name}"));
        Ok(())
    }

    fn reindex(&self, name: &str) -> Result<()> {
        self.calls
            .lock()
            .expect("lock calls")
            .push(format!("reindex:{name}"));
        Ok(())
    }
}
