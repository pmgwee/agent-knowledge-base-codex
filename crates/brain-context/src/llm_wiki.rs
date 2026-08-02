use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use brain_domain::{ProjectId, WorktreeId};
use sha2::{Digest, Sha256};

use crate::{ContextProvider, ContextQuery, ProviderResult, token_count};

const MAX_MARKDOWN_FILES: usize = 10_000;
const MAX_MARKDOWN_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmWikiSourceStatus {
    ReviewedMarkdownVault,
    UnsupportedLiveApi,
}

pub struct LlmWikiProvider {
    project_id: ProjectId,
    worktree_id: WorktreeId,
    vault: PathBuf,
    deadline: Duration,
    max_results: usize,
    max_tokens: usize,
}

impl LlmWikiProvider {
    pub fn new(
        project_id: ProjectId,
        worktree_id: WorktreeId,
        vault: impl AsRef<Path>,
        brain_home: impl AsRef<Path>,
        deadline: Duration,
        max_results: usize,
        max_tokens: usize,
    ) -> Result<Self> {
        validate_llm_wiki_vault(brain_home.as_ref(), vault.as_ref())?;
        ensure!(
            deadline > Duration::ZERO && deadline <= Duration::from_millis(300),
            "LLM Wiki deadline must not exceed 300 ms"
        );
        ensure!(
            max_results > 0 && max_results <= 3,
            "LLM Wiki result limit must not exceed three"
        );
        ensure!(
            max_tokens > 0 && max_tokens <= 600,
            "LLM Wiki token limit must not exceed 600"
        );
        Ok(Self {
            project_id,
            worktree_id,
            vault: fs::canonicalize(vault.as_ref())?,
            deadline,
            max_results,
            max_tokens,
        })
    }

    pub const fn source_status(&self) -> LlmWikiSourceStatus {
        LlmWikiSourceStatus::ReviewedMarkdownVault
    }

    pub fn source_version(&self) -> Result<String> {
        let mut digest = Sha256::new();
        for path in markdown_files(&self.vault)? {
            let metadata = fs::metadata(&path)?;
            let relative = path.strip_prefix(&self.vault)?;
            digest.update(relative.to_string_lossy().replace('\\', "/").as_bytes());
            digest.update(metadata.len().to_le_bytes());
            let modified = metadata
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            digest.update(modified.as_nanos().to_le_bytes());
        }
        Ok(hex::encode(digest.finalize()))
    }

    pub fn search_blocking(&self, query: &ContextQuery) -> Result<Vec<ProviderResult>> {
        ensure!(
            query.project_id == self.project_id,
            "LLM Wiki project scope mismatch"
        );
        ensure!(
            query.worktree_id == self.worktree_id,
            "LLM Wiki worktree scope mismatch"
        );
        let prompt = query.prompt.as_deref().unwrap_or_default();
        let terms = search_terms(prompt, &query.paths);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let started = Instant::now();
        let mut matches = Vec::new();
        for path in markdown_files(&self.vault)? {
            if started.elapsed() >= self.deadline {
                break;
            }
            let relative = path.strip_prefix(&self.vault)?;
            let relative_text = relative.to_string_lossy().replace('\\', "/");
            let lower_path = relative_text.to_lowercase();
            if lower_path
                .split('/')
                .any(|component| matches!(component, "sessions" | "transcripts"))
            {
                continue;
            }
            let metadata = fs::metadata(&path)?;
            if metadata.len() > MAX_MARKDOWN_BYTES {
                continue;
            }
            let contents = fs::read_to_string(&path)?;
            let lower = contents.to_lowercase();
            let hits = terms
                .iter()
                .filter(|term| lower.contains(term.as_str()) || lower_path.contains(term.as_str()))
                .count();
            let relevance = hits as f64 / terms.len() as f64;
            if relevance < 0.20 {
                continue;
            }
            let source_date = metadata.modified().ok().map(time::OffsetDateTime::from);
            matches.push(ProviderResult {
                provider: "llm_wiki".to_owned(),
                project_id: self.project_id,
                worktree_id: Some(self.worktree_id),
                title: document_title(&contents, relative),
                content: excerpt_around(&contents, &terms, 1_600),
                source_uri: relative_text.to_string(),
                source_date,
                observed_at: time::OffsetDateTime::now_utc(),
                trust: "external_document".to_owned(),
                relevance,
                git_head: None,
            });
        }
        matches.sort_by(|left, right| {
            right
                .relevance
                .total_cmp(&left.relevance)
                .then_with(|| right.source_date.cmp(&left.source_date))
                .then_with(|| left.source_uri.cmp(&right.source_uri))
        });
        let mut seen = HashSet::new();
        matches.retain(|item| seen.insert(item.source_uri.to_lowercase()));
        matches.truncate(self.max_results);
        while token_count(&rendered_content(&matches)) > self.max_tokens {
            if matches.len() > 1 {
                matches.pop();
            } else if let Some(item) = matches.first_mut() {
                if item.content.chars().count() <= 80 {
                    matches.clear();
                } else {
                    item.content = item
                        .content
                        .chars()
                        .take(item.content.chars().count() / 2)
                        .collect();
                }
            } else {
                break;
            }
        }
        Ok(matches)
    }
}

#[async_trait]
impl ContextProvider for LlmWikiProvider {
    async fn retrieve(&self, query: &ContextQuery) -> Result<Vec<ProviderResult>> {
        let provider = Self {
            project_id: self.project_id,
            worktree_id: self.worktree_id,
            vault: self.vault.clone(),
            deadline: self.deadline,
            max_results: self.max_results,
            max_tokens: self.max_tokens,
        };
        let query = query.clone();
        tokio::task::spawn_blocking(move || provider.search_blocking(&query)).await?
    }
}

pub fn validate_llm_wiki_vault(brain_home: &Path, vault: &Path) -> Result<()> {
    ensure!(vault.is_dir(), "LLM Wiki Markdown vault does not exist");
    let home = canonical_or_absolute(brain_home)?;
    let vault = fs::canonicalize(vault)?;
    ensure!(
        !same_or_child(&vault, &home) && !same_or_child(&home, &vault),
        "LLM Wiki vault must be separate from canonical brain paths"
    );
    Ok(())
}

fn markdown_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if entry.file_name() != ".obsidian" {
                    pending.push(entry.path());
                }
            } else if file_type.is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
            {
                files.push(entry.path());
                ensure!(
                    files.len() <= MAX_MARKDOWN_FILES,
                    "LLM Wiki vault exceeds file scan limit"
                );
            }
        }
    }
    files.sort();
    Ok(files)
}

fn search_terms(prompt: &str, paths: &[String]) -> Vec<String> {
    let mut terms = prompt
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .chain(paths.iter().flat_map(|path| {
            path.split(|character: char| !character.is_alphanumeric() && character != '_')
        }))
        .map(str::to_lowercase)
        .filter(|term| term.len() >= 3)
        .take(24)
        .collect::<Vec<_>>();
    terms.sort();
    terms.dedup();
    terms
}

fn document_title(contents: &str, path: &Path) -> String {
    contents
        .lines()
        .find_map(|line| line.trim().strip_prefix("# ").map(str::trim))
        .filter(|title| !title.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        })
}

fn excerpt_around(contents: &str, terms: &[String], max_chars: usize) -> String {
    let collapsed = contents.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = collapsed.to_lowercase();
    let position = terms
        .iter()
        .filter_map(|term| lower.find(term))
        .min()
        .unwrap_or(0);
    let start = position.saturating_sub(max_chars / 4);
    collapsed.chars().skip(start).take(max_chars).collect()
}

fn rendered_content(items: &[ProviderResult]) -> String {
    items
        .iter()
        .map(|item| format!("{} {} {}", item.title, item.content, item.source_uri))
        .collect::<Vec<_>>()
        .join("\n")
}

fn canonical_or_absolute(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        fs::canonicalize(path).map_err(Into::into)
    } else if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()?
            .join(path)
            .canonicalize()
            .with_context(|| format!("resolve {}", path.display()))
    }
}

fn same_or_child(candidate: &Path, root: &Path) -> bool {
    let candidate = candidate
        .to_string_lossy()
        .replace('/', "\\")
        .to_lowercase();
    let root = root.to_string_lossy().replace('/', "\\").to_lowercase();
    candidate == root
        || candidate
            .strip_prefix(&root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}
