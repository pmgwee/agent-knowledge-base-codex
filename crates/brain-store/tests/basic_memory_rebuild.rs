use std::sync::Mutex;

use anyhow::Result;
use brain_domain::ProjectId;
use brain_store::{BasicMemoryCli, BasicMemoryIndexer, BasicMemoryState, ProjectionReport};

#[test]
fn disposable_basic_memory_index_rebuilds_from_the_manifest() {
    let temp = tempfile::tempdir().expect("create Basic Memory fixture");
    let project = ProjectId(uuid::Uuid::now_v7());
    let project_root = temp.path().join("project");
    let generation_root = project_root
        .join("generated")
        .join("generations")
        .join("fixture");
    std::fs::create_dir_all(&generation_root).expect("create generation root");
    let projection = ProjectionReport {
        project_id: project,
        generation: "fixture-generation".to_owned(),
        generation_root: generation_root.clone(),
        manifest_path: project_root.join("generated").join("current.json"),
        file_count: 7,
    };
    let runner = FixtureCli::available();
    let report = BasicMemoryIndexer::new(&runner).rebuild(&projection);

    assert_eq!(report.state, BasicMemoryState::Ready);
    assert_eq!(report.projected_files, 7);
    assert!(report.index_root.ends_with("basic-memory"));
    assert_eq!(runner.calls.lock().expect("lock calls").len(), 2);

    runner.calls.lock().expect("lock calls").clear();
    let rebuilt = BasicMemoryIndexer::new(&runner).rebuild(&projection);
    assert_eq!(rebuilt.state, BasicMemoryState::Ready);
    assert_eq!(rebuilt.projected_files, 7);
    assert_eq!(rebuilt.index_root, report.index_root);
    assert_eq!(runner.calls.lock().expect("lock calls").len(), 2);
    assert!(
        generation_root.is_dir(),
        "canonical Markdown was not deleted"
    );
}

#[test]
fn unavailable_basic_memory_is_degraded_not_fatal() {
    let temp = tempfile::tempdir().expect("create fixture");
    let project = ProjectId(uuid::Uuid::now_v7());
    let project_root = temp.path().join("project");
    let projection = ProjectionReport {
        project_id: project,
        generation: "fixture-generation".to_owned(),
        generation_root: project_root
            .join("generated")
            .join("generations")
            .join("fixture"),
        manifest_path: project_root.join("generated").join("current.json"),
        file_count: 3,
    };
    let runner = FixtureCli::unavailable();
    let report = BasicMemoryIndexer::new(&runner).rebuild(&projection);
    assert_eq!(report.state, BasicMemoryState::Degraded);
    assert_eq!(report.projected_files, 3);
    assert!(
        report
            .reason
            .expect("degraded reason")
            .contains("unavailable")
    );
}

struct FixtureCli {
    version: Result<String, String>,
    calls: Mutex<Vec<String>>,
}

impl FixtureCli {
    fn available() -> Self {
        Self {
            version: Ok("basic-memory 0.22.1".to_owned()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn unavailable() -> Self {
        Self {
            version: Err("basic-memory executable is unavailable".to_owned()),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl BasicMemoryCli for FixtureCli {
    fn version(&self) -> Result<String> {
        self.version.clone().map_err(anyhow::Error::msg)
    }

    fn ensure_project(&self, name: &str, root: &std::path::Path) -> Result<()> {
        self.calls
            .lock()
            .expect("lock calls")
            .push(format!("add:{name}:{}", root.display()));
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
