use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use brain_domain::{ProjectId, WorktreeId};
use rusqlite::{OptionalExtension, params};

use crate::{CoordinationStore, from_ns, timestamp_ns};

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Active,
    Completed,
    Abandoned,
}

impl TaskStatus {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Abandoned => "abandoned",
        }
    }

    fn from_name(value: &str) -> Option<Self> {
        Some(match value {
            "active" => Self::Active,
            "completed" => Self::Completed,
            "abandoned" => Self::Abandoned,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct TaskRecord {
    pub id: uuid::Uuid,
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub title: String,
    pub worktree_path: Option<PathBuf>,
    pub branch: Option<String>,
    pub status: TaskStatus,
    pub created_at: time::OffsetDateTime,
    pub closed_at: Option<time::OffsetDateTime>,
}

impl CoordinationStore {
    pub fn create_task(&mut self, task: &TaskRecord) -> Result<()> {
        ensure!(
            task.project_id == self.project_id,
            "task violates project scope"
        );
        ensure!(!task.title.trim().is_empty(), "task title is empty");
        ensure!(task.title.len() <= 300, "task title exceeds 300 bytes");
        self.connection.execute(
            r#"
            INSERT INTO coordination_tasks(
                task_id, project_id, worktree_id, title, worktree_path,
                branch, status, created_at_ns, closed_at_ns
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                task.id.to_string(),
                task.project_id.0.to_string(),
                task.worktree_id.0.to_string(),
                task.title,
                task.worktree_path
                    .as_ref()
                    .map(|path| path.to_string_lossy().to_string()),
                task.branch,
                task.status.as_str(),
                timestamp_ns(task.created_at)?,
                task.closed_at.map(timestamp_ns).transpose()?,
            ],
        )?;
        Ok(())
    }

    pub fn task(&self, task_id: uuid::Uuid) -> Result<Option<TaskRecord>> {
        self.connection
            .query_row(
                r#"
                SELECT task_id, project_id, worktree_id, title, worktree_path,
                       branch, status, created_at_ns, closed_at_ns
                FROM coordination_tasks
                WHERE project_id = ?1 AND task_id = ?2
                "#,
                params![self.project_id.0.to_string(), task_id.to_string()],
                parse_task,
            )
            .optional()?
            .map(parse_task_record)
            .transpose()
    }

    pub fn tasks(&self, include_closed: bool) -> Result<Vec<TaskRecord>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT task_id, project_id, worktree_id, title, worktree_path,
                   branch, status, created_at_ns, closed_at_ns
            FROM coordination_tasks
            WHERE project_id = ?1 AND (?2 OR status = 'active')
            ORDER BY created_at_ns DESC, task_id DESC
            "#,
        )?;
        let rows = statement.query_map(
            params![self.project_id.0.to_string(), include_closed],
            parse_task,
        )?;
        rows.map(|row| parse_task_record(row?)).collect()
    }

    pub fn close_task(
        &mut self,
        task_id: uuid::Uuid,
        status: TaskStatus,
        closed_at: time::OffsetDateTime,
    ) -> Result<TaskRecord> {
        ensure!(
            status != TaskStatus::Active,
            "closing status must be terminal"
        );
        let changed = self.connection.execute(
            r#"
            UPDATE coordination_tasks
            SET status = ?3, closed_at_ns = ?4
            WHERE project_id = ?1 AND task_id = ?2 AND status = 'active'
            "#,
            params![
                self.project_id.0.to_string(),
                task_id.to_string(),
                status.as_str(),
                timestamp_ns(closed_at)?,
            ],
        )?;
        ensure!(changed == 1, "task is absent or already closed");
        self.task(task_id)?.context("closed task disappeared")
    }
}

type RawTask = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    i64,
    Option<i64>,
);

fn parse_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawTask> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
    ))
}

fn parse_task_record(raw: RawTask) -> Result<TaskRecord> {
    Ok(TaskRecord {
        id: uuid::Uuid::parse_str(&raw.0)?,
        project_id: ProjectId(uuid::Uuid::parse_str(&raw.1)?),
        worktree_id: WorktreeId(uuid::Uuid::parse_str(&raw.2)?),
        title: raw.3,
        worktree_path: raw.4.map(PathBuf::from),
        branch: raw.5,
        status: TaskStatus::from_name(&raw.6).context("stored task status is invalid")?,
        created_at: from_ns(raw.7)?,
        closed_at: raw.8.map(from_ns).transpose()?,
    })
}
