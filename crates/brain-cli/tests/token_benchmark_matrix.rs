use std::collections::{BTreeMap, BTreeSet};

use brain_cli::{BenchmarkCondition, BenchmarkHarness, PlannedSample, plan_matrix};

#[test]
fn matrix_is_seeded_complete_and_position_balanced() {
    let tasks = vec![
        "a".to_owned(),
        "b".to_owned(),
        "c".to_owned(),
        "d".to_owned(),
        "e".to_owned(),
    ];
    let first = plan_matrix(&tasks, 2, 42);
    let second = plan_matrix(&tasks, 2, 42);
    assert_eq!(first, second);
    assert_ne!(first, plan_matrix(&tasks, 2, 43));
    assert_eq!(first.len(), tasks.len() * 2 * 2 * 5);

    let mut sample_ids = BTreeSet::new();
    let mut position_counts = BTreeMap::new();
    for block in first.chunks_exact(5) {
        assert!(
            block
                .iter()
                .all(|sample| sample.pair_id == block[0].pair_id)
        );
        assert_eq!(
            block
                .iter()
                .map(|sample| sample.condition)
                .collect::<BTreeSet<_>>(),
            BenchmarkCondition::ALL.into_iter().collect()
        );
        for (order, sample) in block.iter().enumerate() {
            assert_eq!(sample.order, order as u8);
            assert!(sample_ids.insert(sample.sample_id.as_str()));
            *position_counts
                .entry((sample.condition, sample.order))
                .or_insert(0usize) += 1;
            let encoded = serde_json::to_value(sample).unwrap();
            assert!(encoded.get("block_id").is_some());
            assert!(encoded.get("pair_id").is_none());
        }
    }
    assert_eq!(sample_ids.len(), first.len());
    let counts = position_counts.values().copied().collect::<BTreeSet<_>>();
    assert_eq!(
        counts.len(),
        1,
        "every condition must occupy every position equally"
    );
    assert!(
        first
            .iter()
            .any(|sample| sample.harness == BenchmarkHarness::ClaudeCode)
    );
    assert!(
        first
            .iter()
            .any(|sample| sample.harness == BenchmarkHarness::Codex)
    );
}

#[test]
fn schema_v1_samples_remain_readable() {
    let sample: PlannedSample = serde_json::from_value(serde_json::json!({
        "sample_id": "legacy-off",
        "pair_id": "legacy-pair",
        "task_id": "task",
        "harness": "codex",
        "repeat": 1,
        "condition": "brain_off",
        "order": 0
    }))
    .expect("v1 sample");
    assert_eq!(sample.pair_id, "legacy-pair");
    assert_eq!(sample.condition, BenchmarkCondition::C0);
}
