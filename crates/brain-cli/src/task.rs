use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use brain_coordination::{CloseTaskReport, TaskRecord, TaskWorktreeManager};
use brain_domain::ProjectRegistry;
use brain_service::ServiceLaunchConfig;

pub struct TaskCommands {
    manager: TaskWorktreeManager,
}

impl TaskCommands {
    pub fn open(
        brain_home: impl AsRef<Path>,
        project: &str,
        worktree_parent: Option<PathBuf>,
    ) -> Result<Self> {
        let registry = ProjectRegistry::open(brain_home.as_ref())?;
        let project_id = registry.resolve(project)?;
        let config =
            ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(brain_home.as_ref()))?;
        let project = config.project(Some(project_id))?;
        let worktree_parent = worktree_parent.unwrap_or_else(|| {
            project
                .project_root
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("agent-worktrees")
                .join(
                    project
                        .project_root
                        .file_name()
                        .unwrap_or_else(|| std::ffi::OsStr::new("project")),
                )
        });
        Ok(Self {
            manager: TaskWorktreeManager::new(
                brain_home,
                project_id,
                &project.project_root,
                &project.ledger_path,
                worktree_parent,
            )?,
        })
    }

    pub fn create(&self, title: &str, base: Option<&str>) -> Result<TaskRecord> {
        self.manager
            .create(title, base, time::OffsetDateTime::now_utc())
            .context("create task worktree")
    }

    pub fn list(&self, include_closed: bool) -> Result<Vec<TaskRecord>> {
        self.manager.tasks(include_closed)
    }

    pub fn close(&self, task_id: uuid::Uuid) -> Result<CloseTaskReport> {
        self.manager.close(task_id, time::OffsetDateTime::now_utc())
    }
}
