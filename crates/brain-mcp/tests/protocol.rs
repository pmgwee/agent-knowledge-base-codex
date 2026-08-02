use std::fs;

use brain_domain::ProjectRegistry;
use brain_mcp::{BrainTools, McpServer};
use brain_service::{BrainQueryService, ServiceLaunchConfig, ServiceProjectConfig};
use brain_store::EventLedger;

#[test]
fn initialize_and_tool_discovery_follow_the_stdio_mcp_contract() {
    let fixture = Fixture::new();
    let mut server = fixture.server();

    let initialized = request(
        &mut server,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "fixture", "version": "1"}
            }
        }),
    );
    assert_eq!(initialized["result"]["protocolVersion"], "2025-06-18");
    assert!(initialized["result"]["capabilities"]["tools"].is_object());

    let listed = request(
        &mut server,
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    );
    let tools = listed["result"]["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 9);
    for tool in tools {
        assert!(
            tool["inputSchema"]["required"]
                .as_array()
                .expect("required array")
                .iter()
                .any(|field| field == "project"),
            "{tool}"
        );
    }
}

#[test]
fn tool_calls_return_structured_content_and_missing_scope_is_a_tool_error() {
    let fixture = Fixture::new();
    let mut server = fixture.server();
    initialize(&mut server);

    let missing_scope = request(
        &mut server,
        serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "brain_status", "arguments": {}}
        }),
    );
    assert_eq!(missing_scope["result"]["isError"], true);

    let status = request(
        &mut server,
        serde_json::json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {
                "name": "brain_status",
                "arguments": {"project": fixture.project_id.to_string()}
            }
        }),
    );
    assert_eq!(status["result"]["isError"], false);
    assert_eq!(
        status["result"]["structuredContent"]["project_id"],
        fixture.project_id.to_string()
    );
    assert_eq!(
        status["result"]["structuredContent"]["canonical_retrieval"],
        "sqlite_fts5_available"
    );
}

struct Fixture {
    _temp: tempfile::TempDir,
    brain_home: std::path::PathBuf,
    project_id: uuid::Uuid,
    config: ServiceLaunchConfig,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("temp directory");
        let brain_home = temp.path().join("brain");
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("project root");
        let identity = ProjectRegistry::open(&brain_home)
            .expect("registry")
            .register(&project_root)
            .expect("register");
        let ledger_path = brain_home
            .join("projects")
            .join(identity.project_id.0.to_string())
            .join("ledger.sqlite");
        EventLedger::open(&ledger_path, identity.project_id).expect("ledger");
        let mut config = ServiceLaunchConfig::new(r"\\.\pipe\fixture");
        config.upsert_project(ServiceProjectConfig {
            project_root: identity.root,
            project_id: identity.project_id,
            worktree_id: identity.worktree_id,
            ledger_path,
            claude_sources: Vec::new(),
            codex_sources: Vec::new(),
            hermes_database: None,
        });
        Self {
            _temp: temp,
            brain_home,
            project_id: identity.project_id.0,
            config,
        }
    }

    fn server(&self) -> McpServer {
        let service = BrainQueryService::from_config(&self.brain_home, self.config.clone())
            .expect("query service");
        McpServer::new(BrainTools::new(service))
    }
}

fn initialize(server: &mut McpServer) {
    request(
        server,
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "fixture", "version": "1"}}
        }),
    );
}

fn request(server: &mut McpServer, message: serde_json::Value) -> serde_json::Value {
    let line = server
        .handle_line(&message.to_string())
        .expect("request response");
    assert!(!line.contains('\n'), "stdio message must be one line");
    serde_json::from_str(&line).expect("valid JSON-RPC response")
}
