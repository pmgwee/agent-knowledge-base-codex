use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

fn benchmark_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("benchmarks/second-brain/v2")
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn read_jsonl(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn v2_suite_is_balanced_and_covers_cross_harness_continuation() {
    let root = benchmark_root();
    let suite = read_json(&root.join("suite.json"));
    assert_eq!(suite["schema_version"], 2);
    assert_eq!(suite["suite_id"], "second-brain-five-condition-v2");
    assert_eq!(suite["tasks"].as_array().unwrap().len(), 32);
    assert_eq!(suite["pilot_task_ids"].as_array().unwrap().len(), 12);
    assert_eq!(
        suite["benchmark_labels"]["required"],
        serde_json::json!(["current_state", "candidate"])
    );
    for formula in [
        "net_token_savings_percent",
        "speedup_percent",
        "quality_delta_percentage_points",
    ] {
        assert!(suite["formulas"][formula].as_str().is_some());
    }

    let conditions = suite["conditions"].as_array().unwrap();
    assert_eq!(conditions.len(), 5);
    assert_eq!(
        conditions
            .iter()
            .map(|condition| condition["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["c0", "c1", "c2", "c3", "c4"]
    );

    let contrasts = suite["primary_contrasts"].as_array().unwrap();
    assert_eq!(contrasts.len(), 6);
    assert!(
        contrasts
            .iter()
            .any(|contrast| contrast["id"] == "c4_vs_c0")
    );

    let mut strata = BTreeMap::new();
    let mut episode_directions = BTreeMap::new();
    let mut task_ids = BTreeSet::new();
    for task in suite["tasks"].as_array().unwrap() {
        assert!(task_ids.insert(task["id"].as_str().unwrap()));
        *strata
            .entry(task["stratum"].as_str().unwrap())
            .or_insert(0usize) += 1;
        assert_eq!(task["fixture_commit"].as_str().unwrap().len(), 40);
        assert!(!task["reference_facts"].as_array().unwrap().is_empty());
        assert!(root.join(task["rubric"].as_str().unwrap()).is_file());
        assert!(task["max_turns"].as_u64().unwrap() > 0);
        assert!(task["max_tool_calls"].as_u64().unwrap() > 0);
        assert!(task["timeout_seconds"].as_u64().unwrap() > 0);
        if let Some(episode) = task.get("continuation_episode") {
            let key = format!(
                "{}_to_{}",
                episode["producer_harness"].as_str().unwrap(),
                episode["consumer_harness"].as_str().unwrap()
            );
            *episode_directions.entry(key).or_insert(0usize) += 1;
            assert!(episode["history_fixture"].as_str().is_some());
        }
    }
    assert_eq!(
        strata.values().copied().collect::<BTreeSet<_>>(),
        BTreeSet::from([8])
    );
    assert_eq!(task_ids.len(), 32);
    assert_eq!(episode_directions.get("claude_code_to_codex"), Some(&4));
    assert_eq!(episode_directions.get("codex_to_claude_code"), Some(&4));
}

#[test]
fn result_contract_has_independent_goal_verdicts_and_fixed_gates() {
    let schema = read_json(&benchmark_root().join("result-schema.json"));
    assert_eq!(schema["properties"]["schema_version"]["const"], 2);
    let required = schema["required"].as_array().unwrap();
    for field in [
        "goal_1_tokens",
        "goal_2_speed",
        "goal_3_quality",
        "contrasts",
    ] {
        assert!(required.iter().any(|entry| entry == field));
    }
    assert_eq!(
        schema["$defs"]["goal_status"]["enum"]
            .as_array()
            .unwrap()
            .len(),
        5
    );

    let suite = read_json(&benchmark_root().join("suite.json"));
    assert_eq!(suite["statistics"]["bootstrap_resamples"], 10_000);
    assert_eq!(
        suite["release_gates"]["tokens"]["minimum_savings_percent"],
        10.0
    );
    assert_eq!(
        suite["release_gates"]["speed"]["minimum_speedup_percent"],
        10.0
    );
    assert_eq!(
        suite["release_gates"]["quality"]["non_inferiority_margin_percentage_points"],
        -2.0
    );
    assert_eq!(
        suite["release_gates"]["quality"]["historical_superiority_percentage_points"],
        5.0
    );
}

#[test]
fn gold_set_has_240_disjoint_evidence_backed_cases() {
    let root = benchmark_root().join("retrieval-gold");
    let calibration = read_jsonl(&root.join("calibration.jsonl"));
    let locked = read_jsonl(&root.join("locked-test.jsonl"));
    assert_eq!(calibration.len(), 120);
    assert_eq!(locked.len(), 120);

    let mut ids = BTreeSet::new();
    for (split, records) in [("calibration", &calibration), ("locked_test", &locked)] {
        let mut channels = BTreeMap::new();
        let mut prompt_abstentions = 0usize;
        for record in records {
            assert_eq!(record["split"], split);
            assert!(ids.insert(record["id"].as_str().unwrap().to_owned()));
            *channels
                .entry(record["channel"].as_str().unwrap())
                .or_insert(0usize) += 1;
            assert!(record["as_of"].as_str().unwrap().ends_with('Z'));
            let expect_abstention = record["expect_abstention"].as_bool().unwrap();
            let expected_facts = record["expected_facts"].as_array().unwrap();
            let evidence = record["acceptable_evidence"].as_array().unwrap();
            if expect_abstention {
                assert_eq!(record["channel"], "prompt_push");
                assert!(expected_facts.is_empty());
                assert!(evidence.is_empty());
                prompt_abstentions += 1;
            } else {
                assert!(!expected_facts.is_empty());
                assert!(!evidence.is_empty());
            }
            for item in evidence {
                uuid::Uuid::parse_str(item["event_id"].as_str().unwrap()).unwrap();
                assert!(item["source_offset"].as_u64().unwrap() > 0);
            }
            assert!(record["prohibited_facts"].is_array());
            assert!(record["expect_abstention"].is_boolean());
        }
        assert_eq!(channels.get("session_start"), Some(&40));
        assert_eq!(channels.get("prompt_push"), Some(&40));
        assert_eq!(channels.get("historical_pull"), Some(&40));
        assert_eq!(prompt_abstentions, 20);
    }
    assert_eq!(ids.len(), 240);
}
