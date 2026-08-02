use std::collections::{BTreeMap, HashSet};

use brain_domain::{MemoryRecord, MemoryScope, MemoryStatus, ProjectId};

use crate::authority_rank;

#[derive(Clone, Debug)]
pub struct ResolvedMemorySet {
    pub current: Vec<MemoryRecord>,
    pub historical: Vec<MemoryRecord>,
    pub conflicts: Vec<MemoryConflict>,
}

#[derive(Clone, Debug)]
pub struct MemoryConflict {
    pub subject: String,
    pub records: Vec<MemoryRecord>,
}

impl ResolvedMemorySet {
    pub fn rendered_warning(&self) -> String {
        let mut warnings = Vec::new();
        for conflict in &self.conflicts {
            warnings.push(format!(
                "unresolved conflict for {}: {}",
                conflict.subject,
                conflict
                    .records
                    .iter()
                    .map(|record| record.content.as_str())
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
        }
        if !self.historical.is_empty() {
            warnings.push(format!(
                "historical or lower-authority memory: {}",
                self.historical
                    .iter()
                    .map(|record| record.content.as_str())
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
        }
        warnings.join("; ")
    }
}

pub fn resolve_candidates(
    project_id: ProjectId,
    as_of: time::OffsetDateTime,
    candidates: Vec<MemoryRecord>,
) -> ResolvedMemorySet {
    let mut historical = Vec::new();
    let mut active = candidates
        .into_iter()
        .filter(|record| record.scope == MemoryScope::Project(project_id))
        .filter_map(|record| {
            let valid = record.valid_from <= as_of
                && record.valid_to.is_none_or(|valid_to| as_of < valid_to)
                && !matches!(
                    record.status,
                    MemoryStatus::Invalid | MemoryStatus::Superseded
                );
            if valid {
                Some(record)
            } else {
                historical.push(record);
                None
            }
        })
        .collect::<Vec<_>>();

    let superseded = active
        .iter()
        .flat_map(|record| record.supersedes.iter().copied())
        .collect::<HashSet<_>>();
    let mut groups = BTreeMap::<String, Vec<MemoryRecord>>::new();
    for record in active.drain(..) {
        if superseded.contains(&record.version_id) {
            historical.push(record);
            continue;
        }
        groups.entry(subject_key(&record)).or_default().push(record);
    }

    let mut current = Vec::new();
    let mut conflicts = Vec::new();
    for (subject, mut records) in groups {
        records.sort_by(compare_priority);
        let highest_rank = records
            .first()
            .map(|record| authority_rank(&record.authority))
            .unwrap_or(0);
        let contender_count = records
            .iter()
            .take_while(|record| authority_rank(&record.authority) == highest_rank)
            .count();
        let contenders = records.drain(..contender_count).collect::<Vec<_>>();
        let distinct_content = contenders
            .iter()
            .map(|record| normalize_content(&record.content))
            .collect::<HashSet<_>>();
        if distinct_content.len() > 1 {
            conflicts.push(MemoryConflict {
                subject,
                records: contenders,
            });
        } else if let Some(winner) = contenders.first().cloned() {
            current.push(winner);
            historical.extend(contenders.into_iter().skip(1));
        }
        historical.extend(records);
    }
    current.sort_by_key(subject_key);
    historical.sort_by(|left, right| {
        right
            .recorded_at
            .cmp(&left.recorded_at)
            .then_with(|| right.version_id.cmp(&left.version_id))
    });
    conflicts.sort_by(|left, right| left.subject.cmp(&right.subject));
    ResolvedMemorySet {
        current,
        historical,
        conflicts,
    }
}

fn compare_priority(left: &MemoryRecord, right: &MemoryRecord) -> std::cmp::Ordering {
    authority_rank(&right.authority)
        .cmp(&authority_rank(&left.authority))
        .then_with(|| right.recorded_at.cmp(&left.recorded_at))
        .then_with(|| right.confidence.total_cmp(&left.confidence))
        .then_with(|| right.version_id.cmp(&left.version_id))
}

fn subject_key(record: &MemoryRecord) -> String {
    format!(
        "{}:{}",
        record.kind.as_str(),
        record
            .title
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    )
}

fn normalize_content(content: &str) -> String {
    content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
