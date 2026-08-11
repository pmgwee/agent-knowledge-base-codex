use brain_cli::{BenchmarkCondition, BenchmarkHarness, plan_matrix};

#[test]
fn matrix_is_seeded_adjacent_and_balanced() {
    let tasks = vec![
        "a".to_owned(),
        "b".to_owned(),
        "c".to_owned(),
        "d".to_owned(),
    ];
    let first = plan_matrix(&tasks, 3, 42);
    let second = plan_matrix(&tasks, 3, 42);
    assert_eq!(first, second);
    assert_ne!(first, plan_matrix(&tasks, 3, 43));
    assert_eq!(first.len(), tasks.len() * 3 * 2 * 2);

    for pair in first.chunks_exact(2) {
        assert_eq!(pair[0].pair_id, pair[1].pair_id);
        assert_ne!(pair[0].condition, pair[1].condition);
        assert_eq!(pair[0].order, 0);
        assert_eq!(pair[1].order, 1);
    }
    let control_first = first
        .chunks_exact(2)
        .filter(|pair| pair[0].condition == BenchmarkCondition::BrainOff)
        .count();
    let treatment_first = first.len() / 2 - control_first;
    assert!(control_first.abs_diff(treatment_first) <= 1);
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
