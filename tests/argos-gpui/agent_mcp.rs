use argos_gpui::agent_mcp::{BridgeClient, McpProfile, McpServer, RULES, agent_cli};
use argos_gpui::agent_runner::{AgentCli, AgentRun};
use argos_gpui::{AGENT_BRIDGE_FILE, AgentBridge};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Calls = Arc<Mutex<Vec<(String, Value)>>>;
type Fake = McpServer<Box<dyn FnMut(&str, Value) -> Result<Value, String>>>;

fn server(profile: McpProfile) -> (Fake, Calls) {
    let calls: Calls = Arc::default();
    let seen = calls.clone();
    let server: Fake = McpServer::new(
        profile,
        Box::new(move |op: &str, fields: Value| {
            seen.lock().unwrap().push((op.to_string(), fields.clone()));
            match op {
                "state" => Ok(json!({"mode": "advise", "rovers": [{"id": 1}]})),
                "propose" => Ok(json!({"results": [{"rec_id": "rec-1"}]})),
                "stop" => Err("ARGOS is in Advise mode".into()),
                _ => Ok(json!({})),
            }
        }),
    );
    (server, calls)
}

fn rpc(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

#[test]
fn speaks_mcp_and_forwards_tools_to_the_window() {
    let (mut server, calls) = server(McpProfile::Advise);
    let init = server
        .handle(&rpc(
            1,
            "initialize",
            json!({"protocolVersion": "2025-03-26"}),
        ))
        .unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(init["result"]["serverInfo"]["name"], "argos");
    assert!(
        server
            .handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .is_none()
    );

    let tools = server.handle(&rpc(2, "tools/list", json!({}))).unwrap();
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"get_briefing") && names.contains(&"propose"));
    assert!(!names.contains(&"stop"), "advise hides commands");

    let card =
        json!({"rover": 1, "kind": "attention", "action": {}, "urgency": 2, "reason": "held"});
    let reply = server
        .handle(&rpc(
            3,
            "tools/call",
            json!({"name": "propose", "arguments": {"recommendations": [card]}}),
        ))
        .unwrap();
    assert_eq!(reply["result"]["isError"], false);
    assert!(
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("rec-1")
    );
    assert_eq!(
        calls.lock().unwrap().last().unwrap(),
        &("propose".to_string(), json!({"recommendations": [card]}))
    );

    let briefing = server
        .handle(&rpc(
            4,
            "tools/call",
            json!({"name": "get_briefing", "arguments": {}}),
        ))
        .unwrap();
    let text = briefing["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with(&RULES[..40]));
    assert!(text.contains("\"rovers\":[{\"id\":1}]"));

    let refused = server
        .handle(&rpc(
            5,
            "tools/call",
            json!({"name": "stop", "arguments": {"rover": 1}}),
        ))
        .unwrap();
    assert_eq!(refused["result"]["isError"], true);
    assert!(
        refused["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("operator profile")
    );

    let prompt = server
        .handle(&rpc(6, "prompts/get", json!({"name": "recommend_tick"})))
        .unwrap();
    assert!(
        prompt["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap()
            .contains("Call propose once")
    );
    let rules = server
        .handle(&rpc(7, "resources/read", json!({"uri": "argos://rules"})))
        .unwrap();
    assert_eq!(rules["result"]["contents"][0]["text"], RULES);
    let missing = server.handle(&rpc(8, "nope", json!({}))).unwrap();
    assert_eq!(missing["error"]["code"], -32601);
}

#[test]
fn operator_profile_passes_commands_and_window_refusals_back() {
    let (mut server, calls) = server(McpProfile::Operator);
    let reply = server
        .handle(&rpc(
            1,
            "tools/call",
            json!({"name": "stop", "arguments": {"rover": 1}}),
        ))
        .unwrap();
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(
        reply["result"]["content"][0]["text"],
        "ARGOS is in Advise mode"
    );
    server.handle(&rpc(
        2,
        "tools/call",
        json!({"name": "start_task", "arguments": {"rover": 1, "instruction": "find buckets"}}),
    ));
    assert_eq!(
        calls.lock().unwrap().last().unwrap().1["budget_min"],
        5,
        "default budget"
    );
}

#[test]
fn bridge_client_says_hello_and_reads_through_the_window() {
    let dir = std::env::temp_dir().join(format!("argos-mcp-client-{}", std::process::id()));
    let bridge = AgentBridge::start(&dir, 0).unwrap();
    let window = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut ops = Vec::new();
        while Instant::now() < deadline && ops.len() < 2 {
            for call in bridge.take_calls() {
                let op = format!("{:?}", call.envelope.request);
                call.answer(Ok(json!({"rovers": []})));
                ops.push(op);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        ops
    });
    let mut client = BridgeClient::new(Some(dir.join(AGENT_BRIDGE_FILE)), "test-engine");
    assert_eq!(
        client.call("state", json!({})).unwrap()["rovers"],
        json!([])
    );
    let ops = window.join().unwrap();
    assert!(
        ops[0].contains("Hello") && ops[0].contains("test-engine"),
        "{ops:?}"
    );
    assert!(ops[1].contains("State"), "{ops:?}");
    let gone = client.call("state", json!({})).unwrap_err();
    assert!(gone.contains("ARGOS"), "{gone}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_only_takes_its_own_subcommands() {
    assert_eq!(agent_cli(&["--sim".into()]), None);
    assert_eq!(agent_cli(&[]), None);
    assert_eq!(agent_cli(&["call".into()]), Some(2));
    let missing = std::env::temp_dir().join("argos-no-such-bridge.json");
    assert_eq!(
        agent_cli(&[
            "call".into(),
            "state".into(),
            "--bridge".into(),
            missing.display().to_string()
        ]),
        Some(1)
    );
}

fn args(command: &std::process::Command) -> Vec<String> {
    command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn each_engine_gets_argos_mcp_its_own_way() {
    let dir = std::env::temp_dir().join(format!("argos-cli-{}", std::process::id()));
    let exe = std::path::Path::new("/opt/argos/argos-gpui");
    let bridge = dir.join(AGENT_BRIDGE_FILE);
    let run = AgentRun {
        exe,
        bridge: &bridge,
        profile: McpProfile::Advise,
        dir: &dir,
    };

    let claude = AgentCli::Claude.command(&run).unwrap();
    assert_eq!(claude.get_program(), "claude");
    let claude_args = args(&claude);
    assert!(claude_args.contains(&"--strict-mcp-config".to_string()));
    assert!(claude_args.contains(&"mcp__argos".to_string()));
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("argos-mcp-claude.json")).unwrap())
            .unwrap();
    assert_eq!(
        config["mcpServers"]["argos"]["command"],
        "/opt/argos/argos-gpui"
    );
    assert_eq!(config["mcpServers"]["argos"]["args"][0], "mcp");
    assert_eq!(config["mcpServers"]["argos"]["args"][4], "claude-code");

    let codex = args(&AgentCli::Codex.command(&run).unwrap());
    assert_eq!(&codex[..2], ["exec", "--skip-git-repo-check"]);
    assert!(codex.contains(&"mcp_servers.argos.command=\"/opt/argos/argos-gpui\"".to_string()));
    assert!(
        codex
            .iter()
            .any(|arg| arg.starts_with("mcp_servers.argos.args=[\"mcp\""))
    );

    let opencode = AgentCli::OpenCode.command(&run).unwrap();
    assert_eq!(args(&opencode)[0], "run");
    let config: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("argos-mcp-opencode.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(config["mcp"]["argos"]["type"], "local");
    assert_eq!(config["mcp"]["argos"]["command"][1], "mcp");

    let pi = args(&AgentCli::Pi.command(&run).unwrap());
    assert_eq!(&pi[..3], ["--tools", "bash", "-p"]);
    assert!(pi[3].contains("\"/opt/argos/argos-gpui\" call briefing"));
    let _ = std::fs::remove_dir_all(&dir);
}
