use serde::Deserialize;

pub const SUPPORTED_VERSION: u32 = 1;

pub const MCP_PREFIXES: [&str; 2] = ["/mcp", "/mcp/v3"];
pub const DEFAULT_MCP_SCOPE: &str = "mcp:query";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteConfig {
    pub version: u32,
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub routes: Vec<Route>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub strip_prefix: bool,
    #[serde(default)]
    pub websocket: bool,
    #[serde(default)]
    pub strip_request_headers: Vec<String>,
}

pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Authentication {
    #[default]
    Session,
    Bearer,
    InstanceToken,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            timeout_ms: DEFAULT_TIMEOUT_MS,
            strip_prefix: false,
            websocket: false,
            strip_request_headers: Vec::new(),
        }
    }
}

// serde's `default = "..."` takes a function path, not a const, so this is a
// thin wrapper around DEFAULT_TIMEOUT_MS for the `#[serde(default = ...)]` above.
fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub prefix: String,
    pub upstream: String,
    #[serde(default)]
    pub auth: Authentication,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub strip_prefix: Option<bool>,
    #[serde(default)]
    pub websocket: Option<bool>,
    #[serde(default)]
    pub mcp_scope: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedRoute {
    pub prefix: String,
    pub upstream: String,
    pub auth: Authentication,
    pub timeout_ms: u64,
    pub strip_prefix: bool,
    pub websocket: bool,
    pub mcp_scope: String,
}

impl Route {
    #[must_use]
    pub fn resolve(&self, defaults: &Defaults) -> ResolvedRoute {
        ResolvedRoute {
            prefix: self.prefix.clone(),
            upstream: self.upstream.clone(),
            auth: self.auth,
            timeout_ms: self.timeout_ms.unwrap_or(defaults.timeout_ms),
            strip_prefix: self.strip_prefix.unwrap_or(defaults.strip_prefix),
            websocket: self.websocket.unwrap_or(defaults.websocket),
            mcp_scope: self
                .mcp_scope
                .clone()
                .unwrap_or_else(|| DEFAULT_MCP_SCOPE.to_owned()),
        }
    }
}

impl RouteConfig {
    pub fn parse(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }

    #[must_use]
    pub fn resolved_routes(&self) -> Vec<ResolvedRoute> {
        self.routes
            .iter()
            .map(|r| r.resolve(&self.defaults))
            .collect()
    }
}
