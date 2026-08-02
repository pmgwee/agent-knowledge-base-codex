use brain_domain::{HOOK_MAX_FRAME_BYTES, HookReply};

#[test]
fn frames_over_one_mib_are_rejected_before_pipe_io() {
    let reply = HookReply {
        additional_context: Some("x".repeat(HOOK_MAX_FRAME_BYTES)),
        diagnostics_id: None,
    };

    let error = brain_hook::protocol::encode_frame(&reply).expect_err("oversized frame must fail");

    assert!(error.to_string().contains("exceeds"));
}
