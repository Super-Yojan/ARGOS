//! ICE servers for the FPV peer connection. Credentials stay in memory.

use std::collections::BTreeMap;
use std::fmt;

pub const DEFAULT_STUN: &str = "stun:stun.l.google.com:19302";

#[derive(Clone)]
pub struct TurnServer {
    pub url: String,
    pub username: String,
    pub credential: String,
}

impl fmt::Debug for TurnServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TurnServer")
            .field("url", &self.url)
            .field("username", &self.username)
            .field("credential", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct FpvConfig {
    pub stun_urls: Vec<String>,
    pub turn: Option<TurnServer>,
}

impl Default for FpvConfig {
    fn default() -> Self {
        Self {
            stun_urls: vec![DEFAULT_STUN.to_string()],
            turn: None,
        }
    }
}

impl FpvConfig {
    /// Host candidates only. Used by the local loopback test.
    pub fn host_only() -> Self {
        Self {
            stun_urls: Vec::new(),
            turn: None,
        }
    }

    pub fn from_values(values: &BTreeMap<String, String>) -> Result<Self, String> {
        let stun_urls = match values.get("ARGOS_FPV_STUN") {
            None => vec![DEFAULT_STUN.to_string()],
            Some(raw) => parse_stun_list(raw)?,
        };
        let turn_url = trimmed(values.get("ARGOS_FPV_TURN_URL"));
        let turn_user = trimmed(values.get("ARGOS_FPV_TURN_USERNAME"));
        let turn_credential = trimmed(values.get("ARGOS_FPV_TURN_CREDENTIAL"));
        let any_turn = turn_url.is_some() || turn_user.is_some() || turn_credential.is_some();
        let turn = if any_turn {
            let url = turn_url.ok_or(
                "ARGOS_FPV_TURN_URL is required when a TURN username or credential is set",
            )?;
            if url.contains('@') {
                return Err(
                    "ARGOS_FPV_TURN_URL must not embed credentials; set ARGOS_FPV_TURN_USERNAME and ARGOS_FPV_TURN_CREDENTIAL"
                        .into(),
                );
            }
            if !(url.starts_with("turn:") || url.starts_with("turns:")) {
                return Err("ARGOS_FPV_TURN_URL must start with turn: or turns:".into());
            }
            let username = turn_user
                .ok_or("ARGOS_FPV_TURN_USERNAME is required when ARGOS_FPV_TURN_URL is set")?;
            let credential = turn_credential
                .ok_or("ARGOS_FPV_TURN_CREDENTIAL is required when ARGOS_FPV_TURN_URL is set")?;
            Some(TurnServer {
                url,
                username,
                credential,
            })
        } else {
            None
        };
        Ok(Self { stun_urls, turn })
    }
}

fn trimmed(value: Option<&String>) -> Option<String> {
    value
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn parse_stun_list(raw: &str) -> Result<Vec<String>, String> {
    let mut urls = Vec::new();
    for part in raw.split(',') {
        let url = part.trim();
        if url.is_empty() || url.eq_ignore_ascii_case("none") {
            continue;
        }
        if !(url.starts_with("stun:") || url.starts_with("stuns:")) {
            return Err("ARGOS_FPV_STUN entries must start with stun: or stuns:".into());
        }
        urls.push(url.to_string());
    }
    Ok(urls)
}
