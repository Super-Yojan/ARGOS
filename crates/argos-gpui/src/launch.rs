use std::collections::BTreeMap;

pub const LAUNCH_HELP: &str = "Usage: argos-gpui [--sim | --real]\n\n  --sim        Local Zorvane at tcp/127.0.0.1:7447 (start Zorvane separately)\n  --real       Saved fleet connection from environment/.env (default)\n  -h, --help   Show this help\n\n  argos-gpui mcp   MCP server for agents (see `argos-gpui mcp --help`)\n  argos-gpui call  One agent bridge call from a shell\n";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LaunchMode {
    Sim,
    #[default]
    Real,
}

impl LaunchMode {
    /// None means help. Reject ambiguity before any connection is opened.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>, String> {
        let mut args = args.into_iter();
        let mut mode = None;
        while let Some(arg) = args.next() {
            if arg == "--help" || arg == "-h" {
                return Ok(None);
            }
            let value = if arg == "--sim" {
                "sim".into()
            } else if arg == "--real" {
                "real".into()
            } else if arg == "--mode" {
                args.next().ok_or("--mode requires sim or real")?
            } else if let Some(value) = arg.strip_prefix("--mode=") {
                value.to_string()
            } else {
                return Err(format!("Unknown argument: {arg}"));
            };
            if mode.is_some() {
                return Err("Choose exactly one of --sim or --real".into());
            }
            mode = Some(match value.as_str() {
                "sim" => Self::Sim,
                "real" => Self::Real,
                _ => return Err("--mode must be sim or real".into()),
            });
        }
        Ok(Some(mode.unwrap_or_default()))
    }

    /// Window title, so the operator can tell which fleet this window drives.
    pub fn title(self) -> &'static str {
        match self {
            Self::Sim => "ARGOS · Simulation",
            Self::Real => "ARGOS · Real fleet",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Sim => "Simulation",
            Self::Real => "Real fleet",
        }
    }

    pub(crate) fn apply(self, settings: &mut BTreeMap<String, String>) {
        if self == Self::Sim {
            for (key, value) in [
                ("ARGOS_ZENOH_ENDPOINT", "tcp/127.0.0.1:7447"),
                ("ARGOS_ZENOH_PREFIX", "terra/rover"),
                ("ARGOS_ZENOH_USER", ""),
                ("ARGOS_ZENOH_PASSWORD", ""),
            ] {
                settings.insert(key.into(), value.into());
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/argos-gpui/unit/launch_tests.rs"]
mod tests;
