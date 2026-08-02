use brain_domain::{Harness, MemoryRecord};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Citation {
    pub key: String,
    pub detail: String,
}

impl Citation {
    pub fn event(
        event_id: uuid::Uuid,
        harness: &Harness,
        observed_at: time::OffsetDateTime,
    ) -> Self {
        Self {
            key: format!("event:{event_id}"),
            detail: format!("{} at {}", harness.as_str(), observed_at),
        }
    }

    pub fn memory(memory: &MemoryRecord) -> Self {
        let evidence = memory
            .evidence_ids
            .iter()
            .map(|id| format!("event:{id}"))
            .collect::<Vec<_>>()
            .join(",");
        Self {
            key: format!("memory:{}", memory.version_id),
            detail: format!("{}; evidence=[{evidence}]", memory.authority.as_str()),
        }
    }

    pub fn live(head: Option<&str>, observed_at: time::OffsetDateTime) -> Self {
        Self {
            key: format!("live:git-status@{}", head.unwrap_or("unknown")),
            detail: format!("read-only worktree inspection at {observed_at}"),
        }
    }

    pub fn provider(provider: &str, source_uri: &str, observed_at: time::OffsetDateTime) -> Self {
        Self {
            key: format!("provider:{provider}:{}", compact(source_uri, 240)),
            detail: format!("external source observed at {observed_at}"),
        }
    }

    pub fn render(&self) -> String {
        format!("{} ({})", self.key, self.detail)
    }
}

fn compact(value: &str, max: usize) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}
