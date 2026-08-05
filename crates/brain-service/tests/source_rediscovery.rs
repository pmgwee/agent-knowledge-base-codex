use std::fs;
use std::path::{Path, PathBuf};

use brain_domain::{ProjectId, WorktreeId};
use brain_service::{
    ServiceProjectConfig, TranscriptRoots, apply_discovered_sources, discover_new_sources,
};

/// Write a Claude transcript whose recorded working directory is `cwd`.
fn write_claude_transcript(path: &Path, cwd: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("transcript directory");
    }
    let line = serde_json::json!({
        "type": "user",
        "cwd": cwd.to_string_lossy(),
        "message": { "role": "user", "content": [{ "type": "text", "text": text }] }
    });
    fs::write(path, format!("{line}\n")).expect("write transcript");
}

fn project(root: &Path, ledger: &Path, claude_sources: Vec<PathBuf>) -> ServiceProjectConfig {
    ServiceProjectConfig {
        project_root: root.to_path_buf(),
        project_id: ProjectId(uuid::Uuid::now_v7()),
        worktree_id: WorktreeId(uuid::Uuid::now_v7()),
        ledger_path: ledger.to_path_buf(),
        claude_sources,
        codex_sources: Vec::new(),
        hermes_database: None,
    }
}

#[test]
fn a_session_created_after_registration_is_discovered() {
    // The defect this guards: `brain register` records the transcripts that exist at that
    // moment. Every session opened afterwards writes to the same tree but never enters the
    // stored list, so the project silently stops accumulating memory.
    let temp = tempfile::tempdir().expect("temp");
    let project_root = temp.path().join("project");
    fs::create_dir_all(&project_root).expect("project root");
    let claude_root = temp.path().join("claude-projects");

    let at_registration = claude_root.join("encoded-name").join("session-one.jsonl");
    write_claude_transcript(&at_registration, &project_root, "first session");

    let config = project(
        &project_root,
        &temp.path().join("ledger.sqlite"),
        vec![fs::canonicalize(&at_registration).expect("canonical")],
    );
    let roots = TranscriptRoots {
        claude_projects_root: Some(claude_root.clone()),
        codex_sessions_root: None,
    };

    // Nothing new yet — the steady state must stay quiet.
    let found = discover_new_sources(&config, &roots).expect("discover");
    assert!(
        found.is_empty(),
        "a fully-registered project must discover nothing, got {found:?}"
    );

    // A new session opens after registration.
    let after_registration = claude_root.join("encoded-name").join("session-two.jsonl");
    write_claude_transcript(&after_registration, &project_root, "second session");

    let found = discover_new_sources(&config, &roots).expect("discover");
    assert_eq!(found.claude.len(), 1, "the new session must be found");
    assert!(
        found.claude[0].ends_with("session-two.jsonl"),
        "expected session-two, got {:?}",
        found.claude[0]
    );
}

#[test]
fn discovery_never_crosses_a_project_boundary() {
    // Cross-project isolation is a locked release criterion. Rediscovery walks a shared
    // transcript tree, so it must attribute by the transcript's own recorded cwd rather
    // than by proximity in the directory layout.
    let temp = tempfile::tempdir().expect("temp");
    let project_a = temp.path().join("project-a");
    let project_b = temp.path().join("project-b");
    fs::create_dir_all(&project_a).expect("a");
    fs::create_dir_all(&project_b).expect("b");
    let claude_root = temp.path().join("claude-projects");

    // Both transcripts live in the same discovery tree.
    write_claude_transcript(
        &claude_root.join("mixed").join("belongs-to-a.jsonl"),
        &project_a,
        "work in A",
    );
    write_claude_transcript(
        &claude_root.join("mixed").join("belongs-to-b.jsonl"),
        &project_b,
        "work in B",
    );

    let roots = TranscriptRoots {
        claude_projects_root: Some(claude_root),
        codex_sessions_root: None,
    };

    let config_a = project(&project_a, &temp.path().join("a.sqlite"), Vec::new());
    let found_a = discover_new_sources(&config_a, &roots).expect("discover a");
    assert_eq!(found_a.claude.len(), 1, "project A claims exactly its own");
    assert!(found_a.claude[0].ends_with("belongs-to-a.jsonl"));

    let config_b = project(&project_b, &temp.path().join("b.sqlite"), Vec::new());
    let found_b = discover_new_sources(&config_b, &roots).expect("discover b");
    assert_eq!(found_b.claude.len(), 1, "project B claims exactly its own");
    assert!(found_b.claude[0].ends_with("belongs-to-b.jsonl"));
}

#[test]
fn a_transcript_from_an_unrelated_directory_is_ignored() {
    let temp = tempfile::tempdir().expect("temp");
    let project_root = temp.path().join("project");
    let elsewhere = temp.path().join("elsewhere");
    fs::create_dir_all(&project_root).expect("project");
    fs::create_dir_all(&elsewhere).expect("elsewhere");
    let claude_root = temp.path().join("claude-projects");

    write_claude_transcript(
        &claude_root.join("other").join("unrelated.jsonl"),
        &elsewhere,
        "unrelated work",
    );

    let config = project(
        &project_root,
        &temp.path().join("ledger.sqlite"),
        Vec::new(),
    );
    let roots = TranscriptRoots {
        claude_projects_root: Some(claude_root),
        codex_sessions_root: None,
    };

    let found = discover_new_sources(&config, &roots).expect("discover");
    assert!(
        found.is_empty(),
        "a transcript from another directory must not be claimed, got {found:?}"
    );
}

#[test]
fn applying_discoveries_appends_without_disturbing_existing_sources() {
    // Cursors are keyed by source, so an existing entry must never be reordered or
    // replaced — that would orphan its cursor and re-ingest captured evidence.
    let temp = tempfile::tempdir().expect("temp");
    let project_root = temp.path().join("project");
    fs::create_dir_all(&project_root).expect("project");
    let claude_root = temp.path().join("claude-projects");

    let existing = claude_root.join("enc").join("existing.jsonl");
    write_claude_transcript(&existing, &project_root, "existing");
    let existing = fs::canonicalize(&existing).expect("canonical");

    let mut config = project(
        &project_root,
        &temp.path().join("ledger.sqlite"),
        vec![existing.clone()],
    );

    let fresh = claude_root.join("enc").join("fresh.jsonl");
    write_claude_transcript(&fresh, &project_root, "fresh");

    let roots = TranscriptRoots {
        claude_projects_root: Some(claude_root),
        codex_sessions_root: None,
    };
    let found = discover_new_sources(&config, &roots).expect("discover");
    let added = apply_discovered_sources(&mut config, &found);

    assert_eq!(added, 1);
    assert_eq!(config.claude_sources.len(), 2);
    assert_eq!(
        config.claude_sources[0], existing,
        "the pre-existing source must stay first and unchanged"
    );

    // A second pass must be a no-op — discovery is idempotent.
    let found_again = discover_new_sources(&config, &roots).expect("rediscover");
    assert!(
        found_again.is_empty(),
        "already-recorded sources must not be found again, got {found_again:?}"
    );
    let added_again = apply_discovered_sources(&mut config, &found_again);
    assert_eq!(added_again, 0);
    assert_eq!(config.claude_sources.len(), 2);
}

#[test]
fn discovery_is_disabled_when_no_roots_are_configured() {
    let temp = tempfile::tempdir().expect("temp");
    let project_root = temp.path().join("project");
    fs::create_dir_all(&project_root).expect("project");

    let config = project(
        &project_root,
        &temp.path().join("ledger.sqlite"),
        Vec::new(),
    );
    let found = discover_new_sources(&config, &TranscriptRoots::none()).expect("discover");

    assert!(found.is_empty());
}
