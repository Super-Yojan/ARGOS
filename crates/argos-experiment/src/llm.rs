//! Connection to a local, OpenAI-compatible model server (LM Studio by default).
//!
//! Override with environment variables:
//! - `ARGOS_LLM_BASE_URL` (default `http://localhost:1234/v1`, LM Studio's server)
//! - `ARGOS_LLM_MODEL`    (default `qwen/qwen3.8-27b`)

use std::sync::OnceLock;

use rig::Agent;
use rig::AgentBuilder;
use rig::providers::openai::OpenAIConfig;
use std::process;

const DEFAULT_BASE_URL: &str = "http://localhost:9931/v1";
const DEFAULT_MODEL: &str = "LiquidAI/LFM3-Omni-2B-GGUF";

const PREAMBLE: &str = "You are ARGOS's assistant. Answer concisely.";

pub struct LlmConfig {
    pub base_url: String,
    pub model: String,
}

impl LlmConfig {
    pub fn from_env() -> Self {
        Self {
            base_url: std::env::var("ARGOS_LLM_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string()),
            model: std::env::var("ARGOS_LLM_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string()),
        }
    }
}

/// Build an agent that talks to the local server over the Chat Completions API.
pub fn build_agent(config: &LlmConfig) -> Agent {
    // Local servers ignore the API key, but the OpenAI client requires one.
    let client = OpenAIConfig::new("local")
        .with_base_url(config.base_url.clone())
        .client();

    AgentBuilder::new(client.chat(config.model.clone()))
        .preamble(PREAMBLE)
        .build()
}

/// rig's HTTP client (reqwest) needs a tokio runtime, and GPUI runs its own
/// executor, so model requests run on this background runtime.
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("failed to start tokio runtime for LLM requests")

    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run with `cargo test -p argos-experiment -- --ignored` while the server is up.
    #[test]
    #[ignore = "needs a running local model server"]
    fn local_model_replies() {
        let agent = build_agent(&LlmConfig::from_env());
        let mut history = Vec::new();
        let reply = runtime()
            .block_on(agent.chat("Reply with just the word: pong", &mut history))
            .expect("local model request failed")
            .output();
        println!("reply: {reply:?}");
        assert!(!reply.trim().is_empty());
        assert_eq!(
            history.len(),
            2,
            "user prompt and reply are kept for the next turn"
        );
    }
}
