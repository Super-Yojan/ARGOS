use std::{collections::BTreeMap, net::SocketAddr};
#[derive(Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub origin: String,
    pub production: bool,
    pub dev_auth: bool,
    pub demo: bool,
    pub endpoint: String,
    pub prefix: String,
    pub database: String,
    pub web_dir: String,
    pub github_client: String,
    pub github_secret: String,
    pub allowed: BTreeMap<String, String>,
    pub tile_style: Option<String>,
}
impl Config {
    pub fn load() -> Result<Self, String> {
        let get = |key: &str, default: &str| std::env::var(key).unwrap_or_else(|_| default.into());
        let args = std::env::args().collect::<Vec<_>>();
        let production = get("ARGOS_ENV", "development") == "production";
        let dev_auth =
            args.iter().any(|a| a == "--dev") || get("ARGOS_DEV_AUTH", "false") == "true";
        let demo = args.iter().any(|a| a == "--demo") || get("ARGOS_FLEET", "demo") == "demo";
        let mut allowed = BTreeMap::new();
        for (key, role) in [("ARGOS_VIEWERS", "viewer"), ("ARGOS_OPERATORS", "operator")] {
            for id in get(key, "")
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                id.parse::<u64>()
                    .map_err(|_| format!("{key} must contain numeric GitHub IDs"))?;
                allowed.insert(id.into(), role.into());
            }
        }
        let config = Self {
            bind: get("ARGOS_BIND", "127.0.0.1:8080")
                .parse()
                .map_err(|_| "Invalid ARGOS_BIND")?,
            origin: get("ARGOS_ORIGIN", "http://127.0.0.1:5173")
                .trim_end_matches('/')
                .into(),
            production,
            dev_auth,
            demo,
            endpoint: get("ARGOS_ZENOH_ENDPOINT", "tcp/127.0.0.1:7448"),
            prefix: get("ARGOS_ZENOH_PREFIX", "terra/rover"),
            database: get("ARGOS_DATABASE", "sqlite://argos.db"),
            web_dir: get("ARGOS_WEB_DIR", "web/dist"),
            github_client: get("GITHUB_CLIENT_ID", ""),
            github_secret: get("GITHUB_CLIENT_SECRET", ""),
            allowed,
            tile_style: std::env::var("ARGOS_TILE_STYLE")
                .ok()
                .filter(|s| !s.is_empty()),
        };
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<(), String> {
        let u = url::Url::parse(&self.origin).map_err(|_| "Invalid ARGOS_ORIGIN")?;
        if u.host_str().is_none()
            || u.path() != "/"
            || u.query().is_some()
            || u.fragment().is_some()
            || !u.username().is_empty()
            || u.password().is_some()
        {
            return Err("ARGOS_ORIGIN must be an origin without path or credentials".into());
        }
        if self.dev_auth
            && (self.production
                || !self.demo
                || !self.bind.ip().is_loopback()
                || ![Some("127.0.0.1"), Some("localhost"), Some("[::1]")].contains(&u.host_str()))
        {
            return Err(
                "Development identity requires a loopback-only simulated development server".into(),
            );
        }
        if !self.dev_auth
            && (u.scheme() != "https"
                || self.github_client.is_empty()
                || self.github_secret.is_empty()
                || self.allowed.is_empty())
        {
            return Err(
                "Hosted mode requires HTTPS, GitHub OAuth credentials and allowed GitHub IDs"
                    .into(),
            );
        }
        argos_core::Topics::new(&self.prefix).map_err(|e| e.to_string())?;
        if let Some(style) = &self.tile_style {
            let u = url::Url::parse(style).map_err(|_| "Invalid tile style URL")?;
            if u.scheme() != "https" {
                return Err("Tile style URL must use HTTPS".into());
            }
        }
        Ok(())
    }
    pub fn role(&self, id: &str) -> Option<String> {
        if self.dev_auth && id == "dev-operator" {
            Some("operator".into())
        } else {
            self.allowed.get(id).cloned()
        }
    }
}
