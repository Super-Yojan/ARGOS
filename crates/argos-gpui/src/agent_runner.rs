//! Coding-agent CLIs ARGOS can run as its recommendation engine.
//!
//! The operator picks one in the Agent panel. Every tick ARGOS runs it once,
//! headless, with ARGOS's own MCP server (`argos-gpui mcp`) attached, and a
//! short prompt that tells it to read `get_briefing` and propose cards. The
//! CLI brings its own login and model, so ARGOS needs no API key or Python.
//!
//! Each CLI is held to the argos MCP tools: no shell, no file edits, and an
//! environment cleared down to an allow-list, so fleet credentials in ARGOS's
//! own environment never reach the model. Pi is not offered: its only way to
//! reach ARGOS would be its bash tool, which is an unrestricted shell.
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
}

/// Everything a CLI run needs.
pub struct AgentRun<'a> {
    /// This ARGOS binary, which serves `mcp`.
    pub exe: &'a Path,
    /// The window's `agent-bridge.json`.
    pub bridge: &'a Path,
    pub profile: McpProfile,
    /// Where config files for the CLI are written (the session folder).
    pub dir: &'a Path,
    /// What the operator asked the fleet to do, spoken or typed, if anything.
    pub task: Option<&'a str>,
}

const TICK: &str = "You are the recommendation engine for ARGOS, a station where one human \
operator supervises several rovers. Use the argos MCP tools: call get_briefing first and follow it. \
Then either call propose once with up to 3 cards, or answer \"no recommendation\" and why in one \
line. Do not edit files or run other commands.";

/// The tick instruction, plus the operator's task when there is one. The
/// engine plans; every move still reaches the rovers as a card or a checked
/// operator-tool call, never directly.
pub fn tick_text(task: Option<&str>) -> String {
    match task.map(str::trim).filter(|task| !task.is_empty()) {
        None => TICK.to_string(),
        Some(task) => format!(
            "{TICK}\n\nThe operator gave the fleet this task in their own words (often spoken \
             and transcribed, so expect small slips): \"{task}\"\n\
             Plan how the fleet carries it out with the rovers and levels in the briefing. Propose \
             the first steps as cards, up to 3, most urgent first; later steps come on later \
             ticks as the field changes. End your answer with one line that starts with \"Plan:\" \
             (which rover does what, in what order); the operator sees that line. If the task \
             cannot be done with what ARGOS offers, propose nothing and give the reason on that \
             line."
        ),
    }
}

/// Environment every CLI keeps: enough to find its binary, its login and the network.
const BASE_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LC_ALL",
    "TERM",
    "TMPDIR",
    "TMP",
    "TEMP",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "SSL_CERT_FILE",
    "NODE_EXTRA_CA_CERTS",
    // Windows needs these to start any process and find the user profile.
    "SystemRoot",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "PATHEXT",
    "COMSPEC",
];

impl AgentCli {
    pub const ALL: [Self; 3] = [Self::Claude, Self::Codex, Self::OpenCode];

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude-code" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "opencode" => Some(Self::OpenCode),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
        }
    }
    /// The program looked up on PATH.
    pub fn program(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
        }
    }
    /// Name the window shows and logs as the engine.
    pub fn engine(self) -> &'static str {
        match self {
            Self::Claude => "claude-code",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
        }
    }

    /// The CLI's own login and provider settings, on top of [`BASE_ENV`].
    fn auth_env(self) -> &'static [&'static str] {
        match self {
            Self::Claude => &[
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_AUTH_TOKEN",
                "ANTHROPIC_BASE_URL",
                "ANTHROPIC_MODEL",
                "CLAUDE_CODE_OAUTH_TOKEN",
                "CLAUDE_CONFIG_DIR",
                "CLAUDE_CODE_USE_BEDROCK",
                "CLAUDE_CODE_USE_VERTEX",
                "AWS_PROFILE",
                "AWS_REGION",
                "CLOUD_ML_REGION",
                "ANTHROPIC_VERTEX_PROJECT_ID",
            ],
            Self::Codex => &["OPENAI_API_KEY", "OPENAI_BASE_URL", "CODEX_HOME"],
            Self::OpenCode => &[
                "ANTHROPIC_API_KEY",
                "OPENAI_API_KEY",
                "OPENROUTER_API_KEY",
                "GEMINI_API_KEY",
                "GOOGLE_API_KEY",
                "GROQ_API_KEY",
                "OLLAMA_HOST",
            ],
        }
    }

    /// Names of the variables this CLI may see. `ARGOS_AGENT_ENV` adds more,
    /// comma-separated, for a provider not listed here.
    pub fn env_allowed(self) -> Vec<String> {
        let extra = std::env::var("ARGOS_AGENT_ENV").unwrap_or_default();
        BASE_ENV
            .iter()
            .chain(self.auth_env())
            .map(|name| name.to_string())
            .chain(
                extra
                    .split(',')
                    .map(str::trim)
                    .filter(|name| {
                        !name.is_empty() && !name.starts_with("ZENOH") && !name.starts_with("ARGOS")
                    })
                    .map(str::to_string),
            )
            .collect()
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
    /// into `run.dir` first. The child starts from an empty environment plus
    /// [`env_allowed`](Self::env_allowed).
    pub fn command(self, run: &AgentRun) -> std::io::Result<Command> {
        let exe = run.exe.display().to_string();
        let args = self.mcp_args(run);
        let prompt = tick_text(run.task);
        let mut command = Command::new(self.program());
        command.env_clear();
        for name in self.env_allowed() {
            if let Some(value) = std::env::var_os(&name) {
                command.env(name, value);
            }
        }
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
                    .arg(&prompt)
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
                    .args(["exec", "--skip-git-repo-check", "--sandbox", "read-only"])
                    .arg("-c")
                    .arg(format!("mcp_servers.argos.command={}", quoted(&exe)))
                    .arg("-c")
                    .arg(format!("mcp_servers.argos.args={list}"))
                    .arg(&prompt);
            }
            Self::OpenCode => {
                let mut local = vec![exe];
                local.extend(args);
                let config = write(
                    run.dir,
                    "argos-mcp-opencode.json",
                    &json!({
                        "$schema": "https://opencode.ai/config.json",
                        "permission": {"edit": "deny", "bash": "deny", "webfetch": "deny"},
                        "mcp": {"argos": {"type": "local", "command": local, "enabled": true}},
                    }),
                )?;
                command
                    .env("OPENCODE_CONFIG", config)
                    .arg("run")
                    .arg(&prompt);
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

/// Line ARGOS writes into the engine log before each tick's output.
pub const TICK_MARK: &str = "--- tick ";

/// Longest engine reply the Agent panel shows in its conversation.
pub const REPLY_MAX_CHARS: usize = 1_500;

/// What the engine said on its last tick: the log text after the last
/// [`TICK_MARK`] line, trimmed and cut to [`REPLY_MAX_CHARS`]. Claude Code
/// (`-p`) prints only its final answer, so this is that answer; for other
/// engines it is the tail of whatever they printed.
pub fn tick_reply(log: &str) -> Option<String> {
    let start = log
        .rfind(TICK_MARK)
        .map(|at| log[at..].find('\n').map_or(log.len(), |end| at + end + 1))
        .unwrap_or(0);
    let text = log[start..].trim();
    if text.is_empty() {
        return None;
    }
    let count = text.chars().count();
    if count <= REPLY_MAX_CHARS {
        return Some(text.to_string());
    }
    let tail: String = text.chars().skip(count - REPLY_MAX_CHARS).collect();
    Some(format!("…{}", tail.trim_start()))
}

#[cfg(test)]
mod reply_tests {
    use super::*;

    #[test]
    fn takes_only_the_last_tick() {
        let log = "\n--- tick 1 (timer) ---\nold answer\n\n--- tick 2 (operator task) ---\nPlan: rover 1 goes home.\nRover 2 explores.\n";
        assert_eq!(
            tick_reply(log).as_deref(),
            Some("Plan: rover 1 goes home.\nRover 2 explores.")
        );
    }

    #[test]
    fn empty_tick_has_no_reply() {
        assert_eq!(tick_reply("--- tick 3 (timer) ---\n\n"), None);
        assert_eq!(tick_reply(""), None);
    }

    #[test]
    fn long_reply_keeps_the_end() {
        let log = format!("--- tick 1 (timer) ---\n{}END", "a".repeat(3_000));
        let reply = tick_reply(&log).unwrap();
        assert!(reply.starts_with('…') && reply.ends_with("END"));
        assert_eq!(reply.chars().count(), REPLY_MAX_CHARS + 1);
    }
}
