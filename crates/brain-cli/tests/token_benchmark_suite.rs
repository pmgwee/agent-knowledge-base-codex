use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use brain_cli::{BenchmarkTaskStratum, SuiteManifest};

#[test]
fn preregistered_suite_is_balanced_pinned_and_complete() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("benchmarks/token-savings/v1");
    let suite: SuiteManifest =
        serde_json::from_slice(&std::fs::read(root.join("suite.json")).unwrap()).unwrap();
    suite.validate().unwrap();
    assert_eq!(suite.tasks.len(), 32);
    assert!(suite.pilot_task_ids.len() >= 10);
    let mut counts = BTreeMap::new();
    for task in &suite.tasks {
        *counts.entry(task.stratum).or_insert(0usize) += 1;
        assert_eq!(
            task.fixture_commit,
            "737b362aba76f3dfd557e5e38c804f60d28d3be5"
        );
        assert!(
            root.join(&task.rubric).is_file(),
            "missing rubric for {}",
            task.id
        );
        assert!(!task.reference_facts.is_empty());
        assert!(!task.allowed_files.is_empty());
    }
    assert_eq!(counts.len(), BenchmarkTaskStratum::ALL.len());
    assert_eq!(
        counts.values().copied().collect::<BTreeSet<_>>(),
        BTreeSet::from([8])
    );
}
