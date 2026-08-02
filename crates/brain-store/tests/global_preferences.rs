use brain_domain::{Authority, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus, ProjectId};
use brain_store::GlobalPreferenceStore;

#[test]
fn global_store_accepts_only_explicit_preference_records() {
    let mut store = GlobalPreferenceStore::open_in_memory().expect("open global preferences");
    let preference = record(MemoryScope::GlobalPreferences, MemoryKind::Preference);
    store
        .append_preference(&preference)
        .expect("append global preference");
    assert_eq!(
        store.versions(preference.id).expect("read versions").len(),
        1
    );

    let project_fact = record(
        MemoryScope::Project(ProjectId(uuid::Uuid::now_v7())),
        MemoryKind::Fact,
    );
    let error = store
        .append_preference(&project_fact)
        .expect_err("project fact must not auto-promote");
    assert!(error.to_string().contains("global preference"));
}

fn record(scope: MemoryScope, kind: MemoryKind) -> MemoryRecord {
    MemoryRecord {
        id: uuid::Uuid::now_v7(),
        version_id: uuid::Uuid::now_v7(),
        scope,
        worktree_id: None,
        task_id: None,
        kind,
        title: "Preferred test command".to_owned(),
        content: "Use cargo nextest when available".to_owned(),
        valid_from: time::OffsetDateTime::UNIX_EPOCH,
        valid_to: None,
        recorded_at: time::OffsetDateTime::now_utc(),
        confidence: 1.0,
        authority: Authority::HumanCorrection,
        evidence_ids: Vec::new(),
        supersedes: Vec::new(),
        status: MemoryStatus::Current,
    }
}
