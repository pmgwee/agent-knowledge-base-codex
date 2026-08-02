use std::time::Duration;

use brain_service::CaptureServiceConfig;

#[test]
fn capture_timing_rejects_zero_intervals() {
    let invalid_reconciliation = CaptureServiceConfig {
        reconciliation_interval: Duration::ZERO,
        ..CaptureServiceConfig::default()
    };
    assert!(invalid_reconciliation.validate().is_err());

    let invalid_debounce = CaptureServiceConfig {
        watcher_debounce: Duration::ZERO,
        ..CaptureServiceConfig::default()
    };
    assert!(invalid_debounce.validate().is_err());
}

#[test]
fn capture_timing_defaults_match_the_recovery_contract() {
    let config = CaptureServiceConfig::default();
    assert_eq!(config.reconciliation_interval, Duration::from_secs(2));
    assert_eq!(config.watcher_debounce, Duration::from_millis(50));
    config.validate().expect("default timing is valid");
}
