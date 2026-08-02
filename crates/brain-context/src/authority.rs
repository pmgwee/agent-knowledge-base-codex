use brain_domain::Authority;

pub fn authority_rank(authority: &Authority) -> u8 {
    match authority {
        Authority::ExternalDocument => 0,
        Authority::DerivedMemory => 1,
        Authority::AgentCheckpoint => 2,
        Authority::HumanCorrection => 3,
        Authority::RawMechanicalEvidence => 4,
        Authority::LiveState => 5,
    }
}
