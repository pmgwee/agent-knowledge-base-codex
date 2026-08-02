use brain_domain::BrainConfig;

#[test]
fn explicit_brain_home_overrides_the_user_profile_default() {
    let explicit = std::path::PathBuf::from(r"D:\agent-data\brain");
    let profile = std::path::PathBuf::from(r"C:\Users\example");

    let config = BrainConfig::from_paths(Some(explicit.clone()), Some(profile))
        .expect("resolve brain configuration");

    assert_eq!(config.brain_home, explicit);
}

#[test]
fn user_profile_falls_back_to_an_agent_brain_directory() {
    let profile = std::path::PathBuf::from(r"C:\Users\example");

    let config = BrainConfig::from_paths(None, Some(profile.clone()))
        .expect("resolve default brain configuration");

    assert_eq!(config.brain_home, profile.join("AgentBrain"));
}

#[test]
fn missing_home_sources_are_rejected() {
    let error = BrainConfig::from_paths(None, None).expect_err("missing home must fail");

    assert!(error.to_string().contains("USERPROFILE"));
}
