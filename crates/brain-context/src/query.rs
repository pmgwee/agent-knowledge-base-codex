use brain_domain::{ProjectId, WorktreeId};

pub const NORMAL_STARTUP_TOKENS: usize = 1_500;
pub const HARD_MAX_TOKENS: usize = 3_000;

#[derive(Clone, Debug)]
pub struct ContextQuery {
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub native_session_id: Option<String>,
    pub max_tokens: usize,
}

impl ContextQuery {
    pub fn startup(project_id: ProjectId) -> Self {
        Self::for_worktree(project_id, WorktreeId(uuid::Uuid::nil()))
    }

    pub fn for_worktree(project_id: ProjectId, worktree_id: WorktreeId) -> Self {
        Self {
            project_id,
            worktree_id,
            native_session_id: None,
            max_tokens: NORMAL_STARTUP_TOKENS,
        }
    }

    pub fn effective_max_tokens(&self) -> usize {
        self.max_tokens.min(HARD_MAX_TOKENS)
    }
}
