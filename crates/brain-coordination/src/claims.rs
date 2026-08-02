use anyhow::{Context, Result, bail, ensure};
use brain_domain::{ProjectId, WorktreeId};
use rusqlite::{OptionalExtension, params};

use crate::{CoordinationStore, from_ns, timestamp_ns};

const IGNORED_ROOTS: [&str; 10] = [
    ".git",
    ".next",
    "build",
    "coverage",
    "dist",
    "node_modules",
    "target",
    "vendor",
    ".venv",
    "venv",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimKind {
    File,
    Directory,
    Glob,
    Symbol,
}

impl ClaimKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Glob => "glob",
            Self::Symbol => "symbol",
        }
    }

    fn from_name(value: &str) -> Option<Self> {
        Some(match value {
            "file" => Self::File,
            "directory" => Self::Directory,
            "glob" => Self::Glob,
            "symbol" => Self::Symbol,
            _ => return None,
        })
    }
}

#[derive(
    Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Overlap {
    Ignored,
    None,
    Possible,
    Probable,
    Definite,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct PathClaimInput {
    pub kind: ClaimKind,
    pub value: String,
    #[serde(default)]
    pub symbol: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct PathClaim {
    pub id: uuid::Uuid,
    pub project_id: ProjectId,
    pub task_id: uuid::Uuid,
    pub worktree_id: WorktreeId,
    pub kind: ClaimKind,
    pub normalized_value: String,
    pub display_value: String,
    pub symbol: Option<String>,
    pub created_at: time::OffsetDateTime,
    pub released_at: Option<time::OffsetDateTime>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ClaimWarning {
    pub overlap: Overlap,
    pub other: PathClaim,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ClaimResult {
    pub claim: PathClaim,
    pub warnings: Vec<ClaimWarning>,
}

impl CoordinationStore {
    pub fn claim_paths(
        &mut self,
        task_id: uuid::Uuid,
        inputs: Vec<PathClaimInput>,
        now: time::OffsetDateTime,
    ) -> Result<Vec<ClaimResult>> {
        ensure!(!inputs.is_empty(), "at least one path claim is required");
        ensure!(
            inputs.len() <= 100,
            "at most 100 claims may be added at once"
        );
        let task = self.task(task_id)?.context("task does not exist")?;
        ensure!(
            task.status == crate::TaskStatus::Active,
            "task is not active"
        );
        let existing = self.active_claims()?;
        let transaction = self.connection.transaction()?;
        let mut results = Vec::with_capacity(inputs.len());
        for input in inputs {
            let normalized_value = normalize(&input.value)?;
            if input.kind == ClaimKind::Symbol {
                ensure!(
                    input
                        .symbol
                        .as_deref()
                        .is_some_and(|symbol| !symbol.trim().is_empty()),
                    "symbol claims require a symbol"
                );
            }
            let claim = PathClaim {
                id: uuid::Uuid::now_v7(),
                project_id: self.project_id,
                task_id,
                worktree_id: task.worktree_id,
                kind: input.kind,
                normalized_value,
                display_value: input.value,
                symbol: input.symbol.map(|symbol| symbol.trim().to_lowercase()),
                created_at: now,
                released_at: None,
            };
            let warnings = existing
                .iter()
                .chain(results.iter().map(|result: &ClaimResult| &result.claim))
                .filter(|other| other.task_id != task_id)
                .filter_map(|other| {
                    let level = overlap(&claim, other);
                    (level > Overlap::None).then(|| ClaimWarning {
                        overlap: level,
                        other: other.clone(),
                    })
                })
                .collect();
            transaction.execute(
                r#"
                INSERT INTO path_claims(
                    claim_id, project_id, task_id, worktree_id, kind,
                    normalized_value, display_value, symbol, created_at_ns, released_at_ns
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)
                "#,
                params![
                    claim.id.to_string(),
                    claim.project_id.0.to_string(),
                    claim.task_id.to_string(),
                    claim.worktree_id.0.to_string(),
                    claim.kind.as_str(),
                    claim.normalized_value,
                    claim.display_value,
                    claim.symbol,
                    timestamp_ns(claim.created_at)?,
                ],
            )?;
            results.push(ClaimResult { claim, warnings });
        }
        transaction.commit()?;
        Ok(results)
    }

    pub fn active_claims(&self) -> Result<Vec<PathClaim>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT c.claim_id, c.project_id, c.task_id, c.worktree_id, c.kind,
                   c.normalized_value, c.display_value, c.symbol,
                   c.created_at_ns, c.released_at_ns
            FROM path_claims c
            JOIN coordination_tasks t ON t.task_id = c.task_id
            WHERE c.project_id = ?1 AND c.released_at_ns IS NULL AND t.status = 'active'
            ORDER BY c.created_at_ns ASC, c.claim_id ASC
            "#,
        )?;
        let rows = statement.query_map([self.project_id.0.to_string()], parse_claim)?;
        rows.map(|row| parse_claim_record(row?)).collect()
    }

    pub fn release_claim(
        &mut self,
        claim_id: uuid::Uuid,
        task_id: uuid::Uuid,
        now: time::OffsetDateTime,
    ) -> Result<PathClaim> {
        let changed = self.connection.execute(
            r#"
            UPDATE path_claims
            SET released_at_ns = ?4
            WHERE project_id = ?1 AND claim_id = ?2 AND task_id = ?3 AND released_at_ns IS NULL
            "#,
            params![
                self.project_id.0.to_string(),
                claim_id.to_string(),
                task_id.to_string(),
                timestamp_ns(now)?,
            ],
        )?;
        ensure!(changed == 1, "active claim was not found for the task");
        self.connection
            .query_row(
                r#"
                SELECT claim_id, project_id, task_id, worktree_id, kind,
                       normalized_value, display_value, symbol,
                       created_at_ns, released_at_ns
                FROM path_claims WHERE project_id = ?1 AND claim_id = ?2
                "#,
                params![self.project_id.0.to_string(), claim_id.to_string()],
                parse_claim,
            )
            .optional()?
            .context("released claim disappeared")
            .and_then(parse_claim_record)
    }
}

pub fn overlap(left: &PathClaim, right: &PathClaim) -> Overlap {
    if ignored(&left.normalized_value) || ignored(&right.normalized_value) {
        return Overlap::Ignored;
    }
    if left.project_id != right.project_id {
        return Overlap::None;
    }
    if left.kind == ClaimKind::Symbol || right.kind == ClaimKind::Symbol {
        if left.normalized_value == right.normalized_value
            && left.symbol.is_some()
            && left.symbol == right.symbol
        {
            return Overlap::Definite;
        }
        if related(&left.normalized_value, &right.normalized_value) {
            return Overlap::Possible;
        }
        return Overlap::None;
    }
    if left.kind == ClaimKind::Glob || right.kind == ClaimKind::Glob {
        let left_prefix = static_prefix(&left.normalized_value);
        let right_prefix = static_prefix(&right.normalized_value);
        if left_prefix.is_empty() || right_prefix.is_empty() {
            return Overlap::Possible;
        }
        return if related(left_prefix, right_prefix) {
            Overlap::Probable
        } else {
            Overlap::None
        };
    }
    if left.normalized_value == right.normalized_value {
        return Overlap::Definite;
    }
    match (left.kind, right.kind) {
        (ClaimKind::Directory, _) if child_of(&right.normalized_value, &left.normalized_value) => {
            Overlap::Definite
        }
        (_, ClaimKind::Directory) if child_of(&left.normalized_value, &right.normalized_value) => {
            Overlap::Definite
        }
        _ => Overlap::None,
    }
}

fn normalize(value: &str) -> Result<String> {
    let replaced = value.trim().replace('\\', "/");
    ensure!(!replaced.is_empty(), "claim path is empty");
    ensure!(
        !replaced.starts_with('/'),
        "claim path must be repository-relative"
    );
    ensure!(
        !replaced.contains(':'),
        "claim path cannot contain an alternate data stream or drive prefix"
    );
    let mut components = Vec::new();
    for component in replaced.split('/') {
        match component {
            "" | "." => {}
            ".." => bail!("claim path escapes the repository"),
            value => components.push(value.to_lowercase()),
        }
    }
    ensure!(!components.is_empty(), "claim path is empty");
    Ok(components.join("/"))
}

fn ignored(path: &str) -> bool {
    path.split('/')
        .next()
        .is_some_and(|root| IGNORED_ROOTS.contains(&root))
}

fn related(left: &str, right: &str) -> bool {
    left == right || child_of(left, right) || child_of(right, left)
}

fn child_of(candidate: &str, parent: &str) -> bool {
    candidate
        .strip_prefix(parent)
        .is_some_and(|suffix| suffix.starts_with('/'))
}

fn static_prefix(value: &str) -> &str {
    let boundary = value
        .char_indices()
        .find(|(_, character)| matches!(character, '*' | '?' | '[' | '{'))
        .map_or(value.len(), |(index, _)| index);
    value[..boundary].trim_end_matches('/')
}

type RawClaim = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    i64,
    Option<i64>,
);

fn parse_claim(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawClaim> {
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
        row.get(9)?,
    ))
}

fn parse_claim_record(raw: RawClaim) -> Result<PathClaim> {
    Ok(PathClaim {
        id: uuid::Uuid::parse_str(&raw.0)?,
        project_id: ProjectId(uuid::Uuid::parse_str(&raw.1)?),
        task_id: uuid::Uuid::parse_str(&raw.2)?,
        worktree_id: WorktreeId(uuid::Uuid::parse_str(&raw.3)?),
        kind: ClaimKind::from_name(&raw.4).context("stored claim kind is invalid")?,
        normalized_value: raw.5,
        display_value: raw.6,
        symbol: raw.7,
        created_at: from_ns(raw.8)?,
        released_at: raw.9.map(from_ns).transpose()?,
    })
}
