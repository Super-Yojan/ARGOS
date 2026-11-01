//! Local socket an agent's MCP server uses to reach this window.
//!
//! The window listens on `127.0.0.1` only and writes the port and a fresh
//! random token to `agent-bridge.json` in the session folder (owner-only on
//! Unix). Each line in is one JSON request carrying that token; each line out
//! is the reply. Requests are handed to the window's poll loop, so every
//! command runs on the same code path, and with the same checks, as the
//! buttons. The format and the rules are in [`argos_core::agent`].
use argos_core::{
    AGENT_MAX_REQUEST_BYTES, AGENT_PROTOCOL, AgentEnvelope, agent_reply, token_matches,
};
use serde_json::{Value, json};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

/// Default port; `ARGOS_AGENT_PORT` overrides it and `0` picks a free one.
pub const AGENT_PORT: u16 = 7471;
/// Name of the file the MCP server reads to find the window.
pub const AGENT_BRIDGE_FILE: &str = "agent-bridge.json";
/// How long a connection waits for the window to answer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);

/// One request waiting for the window.
pub struct AgentCall {
    pub envelope: AgentEnvelope,
    reply: mpsc::Sender<Value>,
}

impl AgentCall {
    pub fn answer(self, result: Result<Value, String>) {
        let _ = self.reply.send(agent_reply(&self.envelope.id, result));
    }
}

pub struct AgentBridge {
    port: u16,
    file: PathBuf,
    calls: mpsc::Receiver<AgentCall>,
    stop: Arc<AtomicBool>,
    bad_tokens: Arc<AtomicU32>,
}

impl AgentBridge {
    /// Listen on `127.0.0.1:port` and write the bridge file into `dir`.
    pub fn start(dir: &Path, port: u16) -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let token = uuid::Uuid::new_v4().simple().to_string();
        std::fs::create_dir_all(dir)?;
        let file = dir.join(AGENT_BRIDGE_FILE);
        write_private(
            &file,
            &json!({
                "protocol": AGENT_PROTOCOL,
                "host": "127.0.0.1",
                "port": port,
                "token": token,
                "pid": std::process::id(),
            })
            .to_string(),
        )?;
        let (send, calls) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let bad_tokens = Arc::new(AtomicU32::new(0));
        let refused = bad_tokens.clone();
        std::thread::Builder::new()
            .name("argos-agent-bridge".into())
            .spawn(move || accept(listener, token, send, stopping, refused))?;
        Ok(Self {
            port,
            file,
            calls,
            stop,
            bad_tokens,
        })
    }
    pub fn port(&self) -> u16 {
        self.port
    }
    pub fn file(&self) -> &Path {
        &self.file
    }
    /// Requests refused for a wrong or missing token, for example from an
    /// agent still holding the token of an earlier ARGOS run.
    pub fn bad_tokens(&self) -> u32 {
        self.bad_tokens.load(Ordering::Relaxed)
    }
    /// Requests that arrived since the last poll.
    pub fn take_calls(&self) -> Vec<AgentCall> {
        self.calls.try_iter().collect()
    }
}

impl Drop for AgentBridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = std::fs::remove_file(&self.file);
    }
}

fn write_private(path: &Path, text: &str) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(text.as_bytes())
}

fn accept(
    listener: TcpListener,
    token: String,
    send: mpsc::Sender<AgentCall>,
    stop: Arc<AtomicBool>,
    bad_tokens: Arc<AtomicU32>,
) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                let token = token.clone();
                let send = send.clone();
                let stop = stop.clone();
                let bad_tokens = bad_tokens.clone();
                let _ = std::thread::Builder::new()
                    .name("argos-agent-conn".into())
                    .spawn(move || {
                        let _ = serve(stream, &token, &send, &stop, &bad_tokens);
                    });
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

fn serve(
    stream: TcpStream,
    token: &str,
    send: &mpsc::Sender<AgentCall>,
    stop: &AtomicBool,
    bad_tokens: &AtomicU32,
) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(600)))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = Vec::new();
        let read = (&mut reader)
            .take(AGENT_MAX_REQUEST_BYTES as u64 + 1)
            .read_until(b'\n', &mut line)?;
        if read == 0 || stop.load(Ordering::Acquire) {
            return Ok(());
        }
        let too_long = line.len() > AGENT_MAX_REQUEST_BYTES && !line.ends_with(b"\n");
        let envelope = argos_core::parse_agent_line(line.trim_ascii());
        let reply = if too_long {
            agent_reply(
                &Value::Null,
                Err("Request line is longer than 64 KB".into()),
            )
        } else if !token_matches(&envelope.token, token) {
            bad_tokens.fetch_add(1, Ordering::Relaxed);
            agent_reply(
                &envelope.id,
                Err("Wrong or missing token; read it from agent-bridge.json".into()),
            )
        } else {
            let (reply, answer) = mpsc::channel();
            let id = envelope.id.clone();
            if send.send(AgentCall { envelope, reply }).is_err() {
                return Ok(());
            }
            answer
                .recv_timeout(ANSWER_TIMEOUT)
                .unwrap_or_else(|_| agent_reply(&id, Err("ARGOS did not answer in time".into())))
        };
        writer.write_all(reply.to_string().as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        if too_long {
            return Ok(());
        }
    }
}

/// Name prefix of the log an agent command started by ARGOS writes to.
pub const AGENT_LOG_PREFIX: &str = "argos-agent-";

/// An agent process ARGOS started itself (`ARGOS_AGENT_CMD`), so a study run
/// does not depend on someone remembering to start it. Its output goes to
/// `argos-agent-<time>.log` in the session folder; it stops with the window.
pub struct AgentProcess {
    command: String,
    child: std::process::Child,
    log: PathBuf,
    exit: Option<String>,
}

impl AgentProcess {
    /// Run `command` through the platform shell, pointed at `bridge_file`.
    pub fn spawn(command: &str, bridge_file: &Path, dir: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        let log = dir.join(format!("{AGENT_LOG_PREFIX}{stamp}.log"));
        let out = std::fs::File::create(&log)?;
        let err = out.try_clone()?;
        #[cfg(windows)]
        let mut shell = {
            let mut shell = std::process::Command::new("cmd");
            shell.args(["/C", command]);
            shell
        };
        #[cfg(not(windows))]
        let mut shell = {
            let mut shell = std::process::Command::new("sh");
            shell.args(["-c", command]);
            shell
        };
        let child = shell
            .env("ARGOS_AGENT_BRIDGE", bridge_file)
            .env("ARGOS_AGENT_PARENT", std::process::id().to_string())
            .env("PYTHONUNBUFFERED", "1")
            .stdin(std::process::Stdio::null())
            .stdout(out)
            .stderr(err)
            .spawn()?;
        Ok(Self {
            command: command.to_string(),
            child,
            log,
            exit: None,
        })
    }
    pub fn command(&self) -> &str {
        &self.command
    }
    pub fn log(&self) -> &Path {
        &self.log
    }
    /// How it ended, as of the last [`poll`](Self::poll).
    pub fn exited(&self) -> Option<&str> {
        self.exit.as_deref()
    }
    /// `None` while it runs; once it ends, its exit status and last log line.
    pub fn poll(&mut self) -> Option<&str> {
        if self.exit.is_none()
            && let Ok(Some(status)) = self.child.try_wait()
        {
            let last = std::fs::read_to_string(&self.log)
                .ok()
                .and_then(|text| {
                    text.lines()
                        .rev()
                        .find(|line| !line.trim().is_empty())
                        .map(|line| line.trim().chars().take(160).collect::<String>())
                })
                .unwrap_or_else(|| "no output".into());
            self.exit = Some(format!("{status} · {last}"));
        }
        self.exit.as_deref()
    }
}

impl Drop for AgentProcess {
    fn drop(&mut self) {
        if self.exit.is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
