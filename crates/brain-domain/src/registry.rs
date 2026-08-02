use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use atomicwrites::{AllowOverwrite, AtomicFile};

use crate::project::{normalized_path, remote_url};
use crate::{ProjectId, ProjectIdentity, WorktreeId};

const REGISTRY_SCHEMA_VERSION: u32 = 1;

pub struct ProjectRegistry {
    path: PathBuf,
    data: RegistryFile,
}

impl ProjectRegistry {
    pub fn open(brain_home: impl AsRef<Path>) -> Result<Self> {
        fs::create_dir_all(brain_home.as_ref()).with_context(|| {
            format!(
                "failed to create brain home {}",
                brain_home.as_ref().display()
            )
        })?;
        let path = brain_home.as_ref().join("projects.json");
        let data: RegistryFile = if path.exists() {
            let bytes = fs::read(&path)
                .with_context(|| format!("failed to read project registry {}", path.display()))?;
            serde_json::from_slice(&bytes)
                .with_context(|| format!("failed to parse project registry {}", path.display()))?
        } else {
            RegistryFile::default()
        };
        if data.schema_version != REGISTRY_SCHEMA_VERSION {
            bail!(
                "unsupported project registry schema {}; expected {}",
                data.schema_version,
                REGISTRY_SCHEMA_VERSION
            );
        }

        Ok(Self { path, data })
    }

    pub fn register(mut self, root: impl AsRef<Path>) -> Result<ProjectIdentity> {
        let mut identity = ProjectIdentity::inspect(root.as_ref())?;
        let alias = normalized_path(&identity.root)?;

        let key_project_index = self
            .data
            .projects
            .iter()
            .position(|project| project.project_key == identity.project_key);
        let alias_project_index = self
            .data
            .projects
            .iter()
            .position(|project| project.aliases.contains(&alias));
        if key_project_index.is_some()
            && alias_project_index.is_some()
            && key_project_index != alias_project_index
        {
            bail!("project path matches conflicting registry entries");
        }
        let project_index = alias_project_index.or(key_project_index);
        let project = if let Some(index) = project_index {
            &mut self.data.projects[index]
        } else {
            self.data.projects.push(ProjectRecord {
                project_id: ProjectId::new(),
                project_key: identity.project_key.clone(),
                aliases: Vec::new(),
                git_common_dir: identity.git_common_dir.clone(),
                remote: remote_url(&identity.root),
                worktrees: Vec::new(),
            });
            self.data.projects.last_mut().expect("project was inserted")
        };

        if !project.aliases.contains(&alias) {
            project.aliases.push(alias);
        }
        let worktree_id = project
            .worktrees
            .iter()
            .find(|worktree| worktree.worktree_key == identity.worktree_key)
            .map(|worktree| worktree.worktree_id)
            .unwrap_or_else(|| {
                let worktree_id = WorktreeId::new();
                project.worktrees.push(WorktreeRecord {
                    worktree_id,
                    worktree_key: identity.worktree_key.clone(),
                    path: identity.root.clone(),
                    branch: identity.branch.clone(),
                    head: identity.head.clone(),
                });
                worktree_id
            });

        identity.project_id = project.project_id;
        identity.worktree_id = worktree_id;
        self.save()?;
        Ok(identity)
    }

    pub fn add_alias(
        mut self,
        project_id: ProjectId,
        root: impl AsRef<Path>,
    ) -> Result<ProjectIdentity> {
        let mut identity = ProjectIdentity::inspect(root.as_ref())?;
        let alias = normalized_path(&identity.root)?;
        if let Some(owner) = self
            .data
            .projects
            .iter()
            .find(|project| project.aliases.contains(&alias))
            && owner.project_id != project_id
        {
            bail!("path already belongs to project {}", owner.project_id.0);
        }

        let project = self
            .data
            .projects
            .iter_mut()
            .find(|project| project.project_id == project_id)
            .with_context(|| format!("project {} is not registered", project_id.0))?;
        if !project.aliases.contains(&alias) {
            project.aliases.push(alias);
        }
        let worktree_id = project
            .worktrees
            .iter()
            .find(|worktree| worktree.worktree_key == identity.worktree_key)
            .map(|worktree| worktree.worktree_id)
            .unwrap_or_else(|| {
                let worktree_id = WorktreeId::new();
                project.worktrees.push(WorktreeRecord {
                    worktree_id,
                    worktree_key: identity.worktree_key.clone(),
                    path: identity.root.clone(),
                    branch: identity.branch.clone(),
                    head: identity.head.clone(),
                });
                worktree_id
            });

        identity.project_id = project.project_id;
        identity.worktree_id = worktree_id;
        self.save()?;
        Ok(identity)
    }

    pub fn resolve(&self, selector: &str) -> Result<ProjectId> {
        if let Ok(id) = uuid::Uuid::parse_str(selector) {
            let id = ProjectId(id);
            if self
                .data
                .projects
                .iter()
                .any(|project| project.project_id == id)
            {
                return Ok(id);
            }
            bail!("project {} is not registered", id.0);
        }

        let candidate = Path::new(selector);
        let normalized = if candidate.exists() {
            normalized_path(candidate)?
        } else {
            selector.replace('/', "\\").to_lowercase()
        };
        self.data
            .projects
            .iter()
            .find(|project| project.aliases.contains(&normalized))
            .map(|project| project.project_id)
            .with_context(|| format!("project selector {selector:?} is not registered"))
    }

    fn save(&self) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.data)?;
        AtomicFile::new(&self.path, AllowOverwrite)
            .write(|file| {
                file.write_all(&bytes)?;
                file.sync_all()
            })
            .with_context(|| {
                format!(
                    "failed to atomically write project registry {}",
                    self.path.display()
                )
            })
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
struct RegistryFile {
    schema_version: u32,
    projects: Vec<ProjectRecord>,
}

impl Default for RegistryFile {
    fn default() -> Self {
        Self {
            schema_version: REGISTRY_SCHEMA_VERSION,
            projects: Vec::new(),
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
struct ProjectRecord {
    project_id: ProjectId,
    project_key: String,
    aliases: Vec<String>,
    git_common_dir: Option<PathBuf>,
    remote: Option<String>,
    worktrees: Vec<WorktreeRecord>,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct WorktreeRecord {
    worktree_id: WorktreeId,
    worktree_key: String,
    path: PathBuf,
    branch: Option<String>,
    head: Option<String>,
}
