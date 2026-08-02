use brain_context::{ActivationDecision, CodeGraphActivationReport, codegraph_activation_decision};

#[test]
fn provider_stays_disabled_below_token_or_accuracy_gate() {
    let mut report = passing();
    report.provider_targeted_read_tokens = 801;
    assert_eq!(
        codegraph_activation_decision(&report),
        ActivationDecision::KeepDisabled
    );
    report = passing();
    report.provider_accuracy = 0.89;
    assert_eq!(
        codegraph_activation_decision(&report),
        ActivationDecision::KeepDisabled
    );
}

#[test]
fn provider_activates_only_when_every_locked_gate_passes() {
    assert_eq!(
        codegraph_activation_decision(&passing()),
        ActivationDecision::Activate
    );
    let mut report = passing();
    report.all_citations_current = false;
    assert_eq!(
        codegraph_activation_decision(&report),
        ActivationDecision::KeepDisabled
    );
    report = passing();
    report.cold_index_seconds = 2_001.0;
    assert_eq!(
        codegraph_activation_decision(&report),
        ActivationDecision::KeepDisabled
    );
}

fn passing() -> CodeGraphActivationReport {
    CodeGraphActivationReport {
        provider_version: "fixture-1".to_owned(),
        repository_size_class: "medium".to_owned(),
        baseline_targeted_read_tokens: 1_000,
        provider_targeted_read_tokens: 700,
        baseline_accuracy: 0.90,
        provider_accuracy: 0.91,
        all_citations_current: true,
        cold_index_seconds: 100.0,
        average_session_seconds_saved: 10.0,
    }
}
