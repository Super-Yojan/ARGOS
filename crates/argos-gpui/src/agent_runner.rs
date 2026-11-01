//! Coding-agent CLIs ARGOS can run as its recommendation engine.
//!
//! The operator picks one in the Agent panel. Every tick ARGOS runs it once,
//! headless, with ARGOS's own MCP server (`argos-gpui mcp`) attached, and a
//! short prompt that tells it to read `get_briefing` and propose cards. The
//! CLI brings its own login and model, so ARGOS needs no API key or Python.
use crate::agent_mcp::McpProfile;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Which CLI runs the ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentCli {
    Claude,
    Codex,
    OpenCode,
    Pi,
}

/// Everything a CLI run needs.
pub struct AgentRun<'a> {
    /// This ARGOS binary, which serves `mcp` and `call`.
    pub exe: &'a Path,
    /// The window's `agent-bridge.json`.
    pub bridge: &'a Path,
    pub profile: McpProfile,
    /// Where config files for the CLI are written (the session folder).
    pub dir: &'a Path,
}

const TICK: &str = "You are the recommendation engine for ARGOS, a station where one human \
operator supervises several rovers. Use the argos MCP tools: call get_briefing first and follow it. \
Then either call propose once with up to 3 cards, or answer \"no recommendation\" and why in one \
line. Do not edit files or run other commands.";

impl AgentCli {
    pub const ALL: [Self; 4] = [Self::Claude, Self::Codex, Self::OpenCode, Self::Pi];

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude-code" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "opencode" => Some(Self::OpenCode),
            "pi" => Some(Self::Pi),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
            Self::Pi => "Pi",
        }
    }
    /// The program looked up on PATH.
    pub fn program(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Pi => "pi",
        }
    }
    /// Name the window shows and logs as the engine.
    pub fn engine(self) -> &'static str {
        match self {
            Self::Claude => "claude-code",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Pi => "pi",
        }
    }

    fn mcp_args(self, run: &AgentRun) -> Vec<String> {
        vec![
            "mcp".into(),
            "--profile".into(),
            run.profile.wire().into(),
            "--engine".into(),
            self.engine().into(),
            "--bridge".into(),
            run.bridge.display().to_string(),
        ]
    }

    /// The headless command for one tick. Config files it needs are written
    /// into `run.dir` first.
    pub fn command(self, run: &AgentRun) -> std::io::Result<Command> {
        let exe = run.exe.display().to_string();
        let args = self.mcp_args(run);
        let mut command = Command::new(self.program());
        command.env("ARGOS_AGENT_BRIDGE", run.bridge);
        match self {
            Self::Claude => {
                let config = write(
                    run.dir,
                    "argos-mcp-claude.json",
                    &json!({"mcpServers": {"argos": {"command": exe, "args": args}}}),
                )?;
                command
                    .arg("-p")
                    .arg(TICK)
                    .arg("--mcp-config")
                    .arg(config)
                    .arg("--strict-mcp-config")
                    .args(["--allowedTools", "mcp__argos"]);
            }
            Self::Codex => {
                let quoted = |text: &str| serde_json::to_string(text).unwrap_or_default();
                let list = format!(
                    "[{}]",
                    args.iter()
                        .map(|arg| quoted(arg))
                        .collect::<Vec<_>>()
                        .join(",")
                );
                command
                    .args(["exec", "--skip-git-repo-check"])
                    .arg("-c")
                    .arg(format!("mcp_servers.argos.command={}", quoted(&exe)))
                    .arg("-c")
                    .arg(format!("mcp_servers.argos.args={list}"))
                    .arg(TICK);
            }
            Self::OpenCode => {
                let mut local = vec![exe];
                local.extend(args);
                let config = write(
                    run.dir,
                    "argos-mcp-opencode.json",
                    &json!({
                        "$schema": "https://opencode.ai/config.json",
                        "mcp": {"argos": {"type": "local", "command": local, "enabled": true}},
                    }),
                )?;
                command.env("OPENCODE_CONFIG", config).arg("run").arg(TICK);
            }
            Self::Pi => {
                // Pi has no MCP client; it reaches ARGOS through `argos-gpui call`.
                let call = format!("\"{exe}\" call");
                let operator = if run.profile == McpProfile::Operator {
                    " Operator commands (set_level, go_to, stop, ...) use the same form."
                } else {
                    ""
                };
                command.args(["--tools", "bash", "-p"]).arg(format!(
                    "You are the recommendation engine for ARGOS, a station where one human \
                     operator supervises several rovers. Reach ARGOS only with your bash tool: \
                     first run `{call} briefing` and follow it. To put up cards, run \
                     `{call} propose '{{\"recommendations\": [...]}}'` once with up to 3 cards; \
                     `{call} rover '{{\"rover\": 1}}'` reads one rover.{operator} Otherwise answer \
                     \"no recommendation\" and why in one line. Do not edit files or run anything else."
                ));
            }
        }
        Ok(command)
    }
}

fn write(dir: &Path, name: &str, value: &serde_json::Value) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(name);
    std::fs::write(
        &path,
        serde_json::to_string_pretty(value).unwrap_or_default(),
    )?;
    Ok(path)
}
