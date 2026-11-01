//! ARGOS's own MCP server, built into the `argos-gpui` binary.
//!
//! `argos-gpui mcp` speaks MCP over stdio to any agent (Claude Code, Codex,
//! OpenCode, ...) and forwards each tool call to the open ARGOS window over
//! its local bridge, so every command meets the same checks as a button.
//! `argos-gpui call <op> [json]` makes one bridge call from a shell, for
//! agents without MCP. No Python is needed.
use crate::{AGENT_BRIDGE_FILE, session_dir};
use serde_json::{Map, Value, json};
use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

/// The game rules, the bucket rule, the autonomy ladder and how to recommend.
pub const RULES: &str = include_str!("../prompts/rules.md");
/// What the field state and `read_rover` contain.
pub const TOPICS: &str = include_str!("../prompts/topics.md");
/// MCP versions this server speaks, newest first.
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Fixed per match: the rules and what the agent can read.
pub fn system_prompt() -> String {
    format!("{RULES}\n\n{TOPICS}")
}

/// The per-tick instruction plus the field as ARGOS sees it now.
pub fn tick_prompt(state: &Value) -> String {
    format!(
        "Here is the field as ARGOS sees it now (JSON). Decide what, if anything, the operator \
         should do next. Call propose once with up to 3 recommendations, most urgent first. If the \
         fleet needs nothing, do not call propose; say \"no recommendation\" and why in one line.\n\n{state}"
    )
}

/// Everything an agent needs for one tick: rules, what it can read, the field now.
pub fn briefing(state: &Value) -> String {
    format!("{}\n\n{}", system_prompt(), tick_prompt(state))
}

/// `ARGOS_AGENT_BRIDGE`, else `agent-bridge.json` in the session folder.
pub fn bridge_path() -> PathBuf {
    std::env::var_os("ARGOS_AGENT_BRIDGE")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| session_dir().join(AGENT_BRIDGE_FILE))
}

struct Conn {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
}

enum Fail {
    /// The connection broke; worth one reconnect.
    Link(String),
    /// ARGOS answered and refused, or the window is not there.
    Refused(String),
}

/// One connection to the window. Reconnects, and rereads the token, on failure.
pub struct BridgeClient {
    path: PathBuf,
    engine: String,
    conn: Option<Conn>,
    token: String,
    next: u64,
}

impl BridgeClient {
    pub fn new(path: Option<PathBuf>, engine: &str) -> Self {
        Self {
            path: path.unwrap_or_else(bridge_path),
            engine: engine.to_string(),
            conn: None,
            token: String::new(),
            next: 0,
        }
    }

    fn connect(&mut self) -> Result<(), Fail> {
        let info: Value = std::fs::read_to_string(&self.path)
            .map_err(|error| error.to_string())
            .and_then(|text| serde_json::from_str(&text).map_err(|error| error.to_string()))
            .map_err(|error| {
                Fail::Refused(format!(
                    "ARGOS is not running here, or its agent bridge is off (no {}): {error}",
                    self.path.display()
                ))
            })?;
        let port = info["port"].as_u64().unwrap_or_default();
        let host = info["host"].as_str().unwrap_or("127.0.0.1").to_string();
        self.token = info["token"].as_str().unwrap_or_default().to_string();
        let stream = TcpStream::connect((host.as_str(), port as u16)).map_err(|error| {
            Fail::Refused(format!("Cannot reach ARGOS on port {port}: {error}"))
        })?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
        let reader = BufReader::new(stream.try_clone().map_err(|e| Fail::Link(e.to_string()))?);
        self.conn = Some(Conn {
            writer: stream,
            reader,
        });
        self.send("hello", json!({"engine": self.engine}))?;
        Ok(())
    }

    fn send(&mut self, op: &str, fields: Value) -> Result<Value, Fail> {
        self.next += 1;
        let mut line = Map::new();
        line.insert("id".into(), self.next.into());
        line.insert("token".into(), self.token.clone().into());
        line.insert("op".into(), op.into());
        if let Value::Object(fields) = fields {
            line.extend(fields.into_iter().filter(|(_, value)| !value.is_null()));
        }
        let conn = self
            .conn
            .as_mut()
            .ok_or_else(|| Fail::Link("not connected".into()))?;
        let link = |error: io::Error| Fail::Link(error.to_string());
        conn.writer
            .write_all(format!("{}\n", Value::Object(line)).as_bytes())
            .map_err(link)?;
        conn.writer.flush().map_err(link)?;
        let mut reply = String::new();
        if conn.reader.read_line(&mut reply).map_err(link)? == 0 {
            return Err(Fail::Link("ARGOS closed the connection".into()));
        }
        let mut reply: Value =
            serde_json::from_str(&reply).map_err(|error| Fail::Link(error.to_string()))?;
        if reply["ok"] != true {
            let error = reply["error"]
                .as_str()
                .unwrap_or("ARGOS refused the request");
            return Err(Fail::Refused(error.to_string()));
        }
        if let Some(object) = reply.as_object_mut() {
            object.remove("ok");
            object.remove("id");
        }
        Ok(reply)
    }

    /// One request. `Err` carries ARGOS's reason, for the agent to read.
    pub fn call(&mut self, op: &str, fields: Value) -> Result<Value, String> {
        for attempt in 0..2 {
            let result = if self.conn.is_none() {
                self.connect().and_then(|_| self.send(op, fields.clone()))
            } else {
                self.send(op, fields.clone())
            };
            match result {
                Ok(value) => return Ok(value),
                Err(Fail::Refused(error)) => {
                    if error.contains("token") {
                        self.conn = None;
                    }
                    return Err(error);
                }
                Err(Fail::Link(error)) => {
                    self.conn = None;
                    if attempt == 1 {
                        return Err(format!("Lost the connection to ARGOS: {error}"));
                    }
                }
            }
        }
        unreachable!()
    }
}

/// What the agent may ask for. The window's Agent panel still decides.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum McpProfile {
    #[default]
    Advise,
    Operator,
}

impl McpProfile {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "advise" => Some(Self::Advise),
            "operator" => Some(Self::Operator),
            _ => None,
        }
    }
    pub fn wire(self) -> &'static str {
        match self {
            Self::Advise => "advise",
            Self::Operator => "operator",
        }
    }
}

const INSTRUCTIONS: &str = "You are connected to ARGOS, the operator station for a fleet of Terra \
rovers playing the NEXT Competition 2026. Call get_briefing first: it has the rules, the autonomy \
ladder, what each field means and the field right now. In advise mode you suggest moves with \
propose(); the operator accepts or rejects each card, and you can see the decisions in \
list_recommendations().";

const OPERATOR_NOTE: &str = " Operator profile: you may also command rovers (set_level, go_to, \
start_task, stop, ...). ARGOS runs the same checks as for the human operator and tells you why it \
refuses. Raw drive needs request_drive() and the operator's approval; never ask for it just to save \
time. Do not dump balls into buckets: drive close and hand the rover to the operator.";

const PROPOSE_HELP: &str = "Put recommendation cards in front of the operator. Each item: rover \
(id); kind: set_level | go_to | task | handoff_bucket | attention; action, per kind: {\"level\": \
\"explore\", \"budget_min\": 5} | {\"x\": .., \"y\": ..} or {\"latitude\": .., \"longitude\": ..} | \
{\"instruction\": \"..\", \"budget_min\": 5} | {\"object_id\": 7, \"approach_m\": 1.5} | {}; urgency: \
1 FYI, 2 soon, 3 now; reason: under 200 characters, with the numbers that matter; evidence: optional \
list of state fields you used; confidence: optional 0-1; expires_s: optional, default 90. At most 3 \
cards are open. Returns per-item results: rec_id, or why it was refused.";

/// One MCP tool and the bridge op behind it.
struct Tool {
    name: &'static str,
    op: &'static str,
    operator: bool,
    description: &'static str,
    /// (field, JSON type, required, description)
    fields: &'static [(&'static str, &'static str, bool, &'static str)],
}

const ROVER: (&str, &str, bool, &str) = ("rover", "integer", true, "Rover id");

const TOOLS: &[Tool] = &[
    Tool {
        name: "get_briefing",
        op: "briefing",
        operator: false,
        description: "The rules, the autonomy ladder, what each field means, and the field right now. Read it first on every tick.",
        fields: &[],
    },
    Tool {
        name: "get_field_state",
        op: "state",
        operator: false,
        description: "Every rover as ARGOS sees it (pose, level, safety, goal, L4/L5 status, found objects), plus open recommendations, recent decisions, and any drive grant.",
        fields: &[],
    },
    Tool {
        name: "read_rover",
        op: "rover",
        operator: false,
        description: "Raw cached telemetry for one rover (autonomy/status, hardware, outdoor, pose quality, objects, search), each with its age in seconds.",
        fields: &[ROVER],
    },
    Tool {
        name: "list_recommendations",
        op: "recommendations",
        operator: false,
        description: "Open cards and the last 10 decisions, with the operator's reject reasons.",
        fields: &[],
    },
    Tool {
        name: "propose",
        op: "propose",
        operator: false,
        description: PROPOSE_HELP,
        fields: &[(
            "recommendations",
            "array",
            true,
            "Up to 3 cards, most urgent first",
        )],
    },
    Tool {
        name: "withdraw",
        op: "withdraw",
        operator: false,
        description: "Take down one of your open cards that no longer applies.",
        fields: &[("rec_id", "string", true, "The card's rec_id")],
    },
    Tool {
        name: "set_level",
        op: "set_level",
        operator: true,
        description: "Ask a rover for a level: teleop, assisted_teleop, waypoint_direct, waypoint or explore (needs budget_min 3, 5 or 10). Use start_task for L5. The reply means sent; read the rover's level in get_field_state for the result.",
        fields: &[
            ROVER,
            ("level", "string", true, "Wire name of the level"),
            ("budget_min", "integer", false, "L4 budget: 3, 5 or 10"),
        ],
    },
    Tool {
        name: "go_to",
        op: "go_to",
        operator: true,
        description: "Send a goal from L2 or L3: x/y in the rover's local metres, or latitude/longitude when its navigation is outdoor_gps.",
        fields: &[
            ROVER,
            ("x", "number", false, "Local x, metres"),
            ("y", "number", false, "Local y, metres"),
            ("latitude", "number", false, "Degrees"),
            ("longitude", "number", false, "Degrees"),
        ],
    },
    Tool {
        name: "cancel_goal",
        op: "cancel_goal",
        operator: true,
        description: "Cancel the rover's current goal.",
        fields: &[ROVER],
    },
    Tool {
        name: "start_task",
        op: "start_task",
        operator: true,
        description: "Start L5: the rover's on-phone model or MCP agent turns the instruction into checked waypoints. budget_min is 3, 5 or 10 (default 5); agent is builtin or mcp (default: the phone's MCP agent when it has one).",
        fields: &[
            ROVER,
            ("instruction", "string", true, "What the rover should do"),
            ("budget_min", "integer", false, "3, 5 or 10"),
            ("agent", "string", false, "builtin or mcp"),
        ],
    },
    Tool {
        name: "reply_task",
        op: "reply_task",
        operator: true,
        description: "Answer the rover's L5 question, or add guidance to its running task.",
        fields: &[ROVER, ("text", "string", true, "Your answer")],
    },
    Tool {
        name: "cancel_task",
        op: "cancel_task",
        operator: true,
        description: "Cancel the rover's L5 task. It holds at L5.",
        fields: &[ROVER],
    },
    Tool {
        name: "stop",
        op: "stop",
        operator: true,
        description: "Latch the safety stop on one rover. Only the operator can reset it.",
        fields: &[ROVER],
    },
    Tool {
        name: "stop_all",
        op: "stop_all",
        operator: true,
        description: "Latch the safety stop on every rover.",
        fields: &[],
    },
    Tool {
        name: "request_drive",
        op: "request_drive",
        operator: true,
        description: "Ask the operator for up to 60 s of raw drive on one rover (it must be in L0 teleop). Nothing moves until they press Approve drive; poll drive_grant in get_field_state.",
        fields: &[
            ROVER,
            ("seconds", "number", true, "Up to 60"),
            ("reason", "string", true, "Why"),
        ],
    },
    Tool {
        name: "drive",
        op: "drive",
        operator: true,
        description: "Drive during an approved grant for up to 2 s. linear/angular (-1..1) give the direction only (forward/back, left/right); the operator's drive limits set the speed.",
        fields: &[
            ROVER,
            ("linear", "number", true, "-1..1"),
            ("angular", "number", true, "-1..1"),
            ("seconds", "number", false, "Up to 2, default 1"),
        ],
    },
    Tool {
        name: "end_drive",
        op: "end_drive",
        operator: true,
        description: "Release the keys and end your drive grant.",
        fields: &[ROVER],
    },
];

fn tool_json(tool: &Tool) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for (name, kind, needed, about) in tool.fields {
        let mut field = json!({"type": kind, "description": about});
        if *kind == "array" {
            field["items"] = json!({"type": "object"});
        }
        properties.insert((*name).into(), field);
        if *needed {
            required.push(*name);
        }
    }
    json!({
        "name": tool.name,
        "description": tool.description,
        "inputSchema": {"type": "object", "properties": properties, "required": required},
    })
}

/// The bridge op behind `name` in `profile`, if the agent may use it.
fn tool_op(profile: McpProfile, name: &str) -> Option<&'static str> {
    TOOLS
        .iter()
        .find(|tool| tool.name == name && (!tool.operator || profile == McpProfile::Operator))
        .map(|tool| tool.op)
}

/// Run one bridge op; `briefing` is composed here from the field state.
pub fn bridge_op(
    call: &mut dyn FnMut(&str, Value) -> Result<Value, String>,
    op: &str,
    fields: Value,
) -> Result<Value, String> {
    if op == "briefing" {
        let state = call("state", json!({}))?;
        return Ok(Value::String(briefing(&state)));
    }
    call(op, fields)
}

/// MCP over JSON-RPC, with the bridge behind a closure so tests can fake it.
pub struct McpServer<F: FnMut(&str, Value) -> Result<Value, String>> {
    profile: McpProfile,
    call: F,
}

impl<F: FnMut(&str, Value) -> Result<Value, String>> McpServer<F> {
    pub fn new(profile: McpProfile, call: F) -> Self {
        Self { profile, call }
    }

    /// Answer one JSON-RPC message; `None` for notifications.
    pub fn handle(&mut self, message: &Value) -> Option<Value> {
        let id = message.get("id")?.clone();
        let method = message["method"].as_str().unwrap_or_default();
        let params = &message["params"];
        let result = match method {
            "initialize" => {
                let asked = params["protocolVersion"].as_str().unwrap_or_default();
                let version = PROTOCOL_VERSIONS
                    .iter()
                    .find(|version| **version == asked)
                    .unwrap_or(&PROTOCOL_VERSIONS[0]);
                let mut instructions = INSTRUCTIONS.to_string();
                if self.profile == McpProfile::Operator {
                    instructions.push_str(OPERATOR_NOTE);
                }
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}, "prompts": {}, "resources": {}},
                    "serverInfo": {"name": "argos", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": instructions,
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({
                "tools": TOOLS
                    .iter()
                    .filter(|tool| !tool.operator || self.profile == McpProfile::Operator)
                    .map(tool_json)
                    .collect::<Vec<_>>(),
            })),
            "tools/call" => Ok(self.call_tool(
                params["name"].as_str().unwrap_or_default(),
                params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            )),
            "prompts/list" => Ok(json!({"prompts": [{
                "name": "recommend_tick",
                "description": "The game rules, what you can read, and the field right now: run once a minute.",
            }]})),
            "prompts/get" if params["name"] == "recommend_tick" => {
                bridge_op(&mut self.call, "briefing", json!({})).map(|text| {
                    json!({
                        "description": "One recommendation tick",
                        "messages": [{"role": "user", "content": {"type": "text", "text": text}}],
                    })
                })
            }
            "prompts/get" => Err("Unknown prompt".to_string()),
            "resources/list" => Ok(json!({"resources": [
                {"uri": "argos://rules", "name": "rules", "mimeType": "text/markdown",
                 "description": "NEXT 2026 rules, the bucket rule, the L0-L5 ladder, and how to recommend."},
                {"uri": "argos://topics", "name": "topics", "mimeType": "text/markdown",
                 "description": "What get_field_state and read_rover contain, and the Zenoh topics behind them."},
            ]})),
            "resources/read" => match params["uri"].as_str() {
                Some(uri @ "argos://rules") => Ok(resource(uri, RULES)),
                Some(uri @ "argos://topics") => Ok(resource(uri, TOPICS)),
                _ => Err("Unknown resource".to_string()),
            },
            _ => {
                return Some(json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": {"code": -32601, "message": format!("Method not found: {method}")},
                }));
            }
        };
        Some(match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(message) => json!({
                "jsonrpc": "2.0", "id": id,
                "error": {"code": -32602, "message": message},
            }),
        })
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Value {
        let result = match tool_op(self.profile, name) {
            None if TOOLS.iter().any(|tool| tool.name == name) => Err(format!(
                "{name} needs the operator profile (argos-gpui mcp --profile operator)"
            )),
            None => Err(format!("Unknown tool {name}")),
            Some(op) => {
                let mut fields = arguments;
                if op == "start_task" && fields.get("budget_min").is_none_or(Value::is_null) {
                    fields["budget_min"] = 5.into();
                }
                if op == "drive" && fields.get("seconds").is_none_or(Value::is_null) {
                    fields["seconds"] = 1.0.into();
                }
                bridge_op(&mut self.call, op, fields)
            }
        };
        let (text, error) = match result {
            Ok(Value::String(text)) => (text, false),
            Ok(value) => (
                serde_json::to_string_pretty(&value).unwrap_or_default(),
                false,
            ),
            Err(reason) => (reason, true),
        };
        json!({"content": [{"type": "text", "text": text}], "isError": error})
    }

    /// Serve MCP on stdin/stdout until the client closes stdin.
    pub fn run_stdio(&mut self) -> io::Result<()> {
        let stdin = io::stdin();
        let mut stdout = io::stdout().lock();
        for line in stdin.lock().lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let reply = match serde_json::from_str::<Value>(&line) {
                Ok(Value::Array(batch)) => {
                    let replies: Vec<Value> =
                        batch.iter().filter_map(|item| self.handle(item)).collect();
                    (!replies.is_empty()).then(|| Value::Array(replies))
                }
                Ok(message) => self.handle(&message),
                Err(error) => Some(json!({
                    "jsonrpc": "2.0", "id": null,
                    "error": {"code": -32700, "message": error.to_string()},
                })),
            };
            if let Some(reply) = reply {
                writeln!(stdout, "{reply}")?;
                stdout.flush()?;
            }
        }
        Ok(())
    }
}

fn resource(uri: &str, text: &str) -> Value {
    json!({"contents": [{"uri": uri, "mimeType": "text/markdown", "text": text}]})
}

pub const AGENT_CLI_HELP: &str = "Usage:\n  argos-gpui mcp [--profile advise|operator] [--engine NAME] [--bridge FILE]\n      MCP server over stdio for Claude Code, Codex, OpenCode or any MCP client\n  argos-gpui call OP [JSON]\n      One bridge call from a shell, e.g. `argos-gpui call briefing` or\n      `argos-gpui call propose '{\"recommendations\": [...]}'`\n";

/// Handle `mcp` and `call`. `None` means the arguments are for the window.
pub fn agent_cli(args: &[String]) -> Option<i32> {
    let (first, rest) = args.split_first()?;
    let mut profile = McpProfile::Advise;
    let mut engine = None;
    let mut bridge = None;
    let mut positional = Vec::new();
    let mut items = rest.iter();
    while let Some(arg) = items.next() {
        match arg.as_str() {
            "--profile" => match items.next().and_then(|text| McpProfile::parse(text)) {
                Some(value) => profile = value,
                None => return Some(usage("--profile is advise or operator")),
            },
            "--engine" => engine = items.next().cloned(),
            "--bridge" => bridge = items.next().map(PathBuf::from),
            "-h" | "--help" => return Some(usage("")),
            _ => positional.push(arg.clone()),
        }
    }
    match first.as_str() {
        "mcp" => {
            let mut client = BridgeClient::new(bridge, engine.as_deref().unwrap_or("mcp-agent"));
            let mut server =
                McpServer::new(profile, |op: &str, fields: Value| client.call(op, fields));
            Some(match server.run_stdio() {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("argos-gpui mcp: {error}");
                    1
                }
            })
        }
        "call" => {
            let Some(op) = positional.first() else {
                return Some(usage("call needs an op"));
            };
            let fields = match positional.get(1) {
                Some(text) => match serde_json::from_str::<Value>(text) {
                    Ok(value @ Value::Object(_)) => value,
                    _ => return Some(usage("the second argument must be a JSON object")),
                },
                None => json!({}),
            };
            let mut client = BridgeClient::new(bridge, engine.as_deref().unwrap_or("cli-agent"));
            let mut call = |op: &str, fields: Value| client.call(op, fields);
            Some(match bridge_op(&mut call, op, fields) {
                Ok(Value::String(text)) => {
                    println!("{text}");
                    0
                }
                Ok(value) => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&value).unwrap_or_default()
                    );
                    0
                }
                Err(error) => {
                    eprintln!("ARGOS: {error}");
                    1
                }
            })
        }
        _ => None,
    }
}

fn usage(error: &str) -> i32 {
    if error.is_empty() {
        print!("{AGENT_CLI_HELP}");
        0
    } else {
        eprint!("{error}\n{AGENT_CLI_HELP}");
        2
    }
}
