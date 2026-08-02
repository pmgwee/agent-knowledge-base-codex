use brain_domain::{ProjectId, WorktreeId};
use brain_store::{SearchQuery, SearchSourceFilter};

pub const NORMAL_STARTUP_TOKENS: usize = 1_500;
pub const HARD_MAX_TOKENS: usize = 3_000;

#[derive(Clone, Debug)]
pub struct ContextQuery {
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub native_session_id: Option<String>,
    pub prompt: Option<String>,
    pub paths: Vec<String>,
    pub as_of: Option<time::OffsetDateTime>,
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
            prompt: None,
            paths: Vec::new(),
            as_of: None,
            max_tokens: NORMAL_STARTUP_TOKENS,
        }
    }

    pub fn effective_max_tokens(&self) -> usize {
        self.max_tokens.min(HARD_MAX_TOKENS)
    }
}

#[derive(Clone, Debug)]
pub struct RetrievalQuery {
    pub project_id: ProjectId,
    pub text: Option<String>,
    pub occurred: Option<brain_store::TimeRange>,
    pub as_of: Option<time::OffsetDateTime>,
    pub worktree_id: Option<WorktreeId>,
    pub task_id: Option<uuid::Uuid>,
    pub native_session_id: Option<String>,
    pub paths: Vec<String>,
    pub source_filter: SearchSourceFilter,
    pub now: time::OffsetDateTime,
    pub limit: usize,
}

impl RetrievalQuery {
    pub fn text(project_id: ProjectId, text: impl Into<String>, now: time::OffsetDateTime) -> Self {
        Self {
            project_id,
            text: Some(text.into()),
            occurred: None,
            as_of: None,
            worktree_id: None,
            task_id: None,
            native_session_id: None,
            paths: Vec::new(),
            source_filter: SearchSourceFilter::All,
            now,
            limit: 20,
        }
    }

    pub fn between(
        project_id: ProjectId,
        start: time::OffsetDateTime,
        end: time::OffsetDateTime,
        now: time::OffsetDateTime,
    ) -> Self {
        let mut query = Self::text(project_id, "", now);
        query.text = None;
        query.occurred = Some(brain_store::TimeRange { start, end });
        query
    }

    pub fn last_day(project_id: ProjectId, now: time::OffsetDateTime) -> Self {
        Self::between(project_id, now - time::Duration::days(1), now, now)
    }

    pub fn last_week(project_id: ProjectId, now: time::OffsetDateTime) -> Self {
        Self::between(project_id, now - time::Duration::days(7), now, now)
    }

    pub fn last_month(project_id: ProjectId, now: time::OffsetDateTime) -> Self {
        Self::between(project_id, now - time::Duration::days(30), now, now)
    }

    pub fn as_of(mut self, as_of: time::OffsetDateTime) -> Self {
        self.as_of = Some(as_of);
        self
    }

    pub fn for_worktree(mut self, worktree_id: WorktreeId) -> Self {
        self.worktree_id = Some(worktree_id);
        self
    }

    pub fn for_task(mut self, task_id: uuid::Uuid) -> Self {
        self.task_id = Some(task_id);
        self
    }

    pub fn for_session(mut self, native_session_id: impl Into<String>) -> Self {
        self.native_session_id = Some(native_session_id.into());
        self
    }

    pub fn with_paths<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.paths = paths.into_iter().map(Into::into).collect();
        self
    }

    pub fn events_only(mut self) -> Self {
        self.source_filter = SearchSourceFilter::Events;
        self
    }

    pub fn memories_only(mut self) -> Self {
        self.source_filter = SearchSourceFilter::Memories;
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit.min(100);
        self
    }

    pub(crate) fn store_query(&self) -> SearchQuery {
        let mut query = if let Some(text) = &self.text {
            SearchQuery::text(self.project_id, text)
        } else if let Some(range) = self.occurred {
            SearchQuery::between(self.project_id, range.start, range.end)
        } else {
            SearchQuery::between(
                self.project_id,
                time::OffsetDateTime::UNIX_EPOCH,
                self.now + time::Duration::nanoseconds(1),
            )
        };
        query.occurred = self.occurred;
        query.as_of = self.as_of;
        query.worktree_id = self.worktree_id;
        query.task_id = self.task_id;
        query.native_session_id = self.native_session_id.clone();
        query.source_filter = self.source_filter;
        query.with_limit(self.limit.saturating_mul(4).max(20))
    }
}
