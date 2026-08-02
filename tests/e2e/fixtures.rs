use std::path::PathBuf;
use std::sync::Arc;

use brain_adapters::{ClaudeAdapter, NormalizeContext, SourceDescriptor};
use brain_cli::{RegisterOptions, RegistrationResult, register_project};
use brain_service::{
    CaptureBinding, CaptureSupervisor, ClaudeHookHandler, HookPipeServer, HookProjectBinding,
};

pub struct E2eFixture {
    _temp: tempfile::TempDir,
    pub brain_home: PathBuf,
    pub project_root: PathBuf,
    transcript: PathBuf,
    pipe_name: String,
}

impl E2eFixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().expect("create E2E fixture");
        let brain_home = temp.path().join("brain");
        let project_root = temp.path().join("project");
        let transcript = temp.path().join("claude").join("prior-session.jsonl");
        std::fs::create_dir_all(&project_root).expect("create E2E project");
        std::fs::create_dir_all(transcript.parent().expect("transcript parent"))
            .expect("create transcript directory");
        Self {
            _temp: temp,
            brain_home,
            project_root,
            transcript,
            pipe_name: format!(r"\\.\pipe\agent-brain-e2e-{}", uuid::Uuid::now_v7()),
        }
    }

    pub fn register_project(&self) -> RegistrationResult {
        register_project(RegisterOptions {
            brain_home: self.brain_home.clone(),
            project_path: self.project_root.clone(),
            claude_projects_root: None,
            explicit_claude_sources: vec![self.transcript.clone()],
            pipe_name: Some(self.pipe_name.clone()),
        })
        .expect("register E2E project")
    }

    pub fn write_prior_claude_session(&self, task: &str, outcome: &str) {
        let records = [
            serde_json::json!({
                "type": "user",
                "sessionId": "prior-session",
                "uuid": "prior-user-turn",
                "timestamp": "2026-08-01T10:00:00Z",
                "cwd": self.project_root,
                "message": {"content": task}
            }),
            serde_json::json!({
                "type": "assistant",
                "sessionId": "prior-session",
                "uuid": "prior-assistant-turn",
                "timestamp": "2026-08-01T10:01:00Z",
                "cwd": self.project_root,
                "message": {"content": [{"type": "text", "text": outcome}]}
            }),
        ];
        let contents = records
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&self.transcript, contents).expect("write prior Claude session");
    }

    pub async fn capture(&self, registered: &RegistrationResult) {
        let binding = CaptureBinding::new(
            Arc::new(ClaudeAdapter::new(
                self.transcript.parent().expect("transcript parent"),
            )),
            SourceDescriptor::file(&self.transcript),
            NormalizeContext {
                project_id: registered.project_id,
                worktree_id: registered.worktree_id,
                source_schema: "claude-jsonl:e2e".to_owned(),
            },
            &registered.ledger_path,
        );
        CaptureSupervisor::new(vec![binding])
            .expect("create capture supervisor")
            .capture_once()
            .await
            .expect("capture prior Claude session");
    }

    pub async fn start_fresh_claude_session(&self, registered: &RegistrationResult) -> String {
        let handler = ClaudeHookHandler::new(HookProjectBinding {
            project_root: self.project_root.clone(),
            project_id: registered.project_id,
            worktree_id: registered.worktree_id,
            ledger_path: registered.ledger_path.clone(),
        })
        .expect("create hook handler");
        let server_name = self.pipe_name.clone();
        let server = tokio::spawn(async move {
            HookPipeServer::new(server_name)
                .serve_once(move |envelope| async move { handler.handle(&envelope) })
                .await
                .expect("serve SessionStart hook");
        });
        tokio::task::yield_now().await;

        let input = serde_json::to_vec(&serde_json::json!({
            "session_id": "fresh-session",
            "transcript_path": self.transcript,
            "cwd": self.project_root,
            "hook_event_name": "SessionStart",
            "source": "startup"
        }))
        .expect("serialize SessionStart payload");
        let output = brain_hook::invoke(
            brain_hook::HookOptions {
                brain_home: self.brain_home.clone(),
                pipe_name: self.pipe_name.clone(),
                harness: brain_domain::Harness::ClaudeCode,
                event_name: None,
                timeout: std::time::Duration::from_secs(1),
            },
            &input,
        )
        .await;
        server.await.expect("join hook server");
        output
            .pointer("/hookSpecificOutput/additionalContext")
            .and_then(serde_json::Value::as_str)
            .expect("SessionStart additionalContext")
            .to_owned()
    }
}
