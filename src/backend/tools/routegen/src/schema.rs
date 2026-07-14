//! `routes.yaml` schema -- the configurator's input contract, salvaged nearly
//! verbatim from the deleted Router spec (gateway DESIGN section 3.8). One field
//! was dropped from that schema (`websocket_max_lifetime_seconds`): nginx cannot
//! enforce an absolute socket lifetime, so it is enforced downstream instead.

use serde::Deserialize;

/// The only schema version this configurator understands.
pub const SUPPORTED_VERSION: u32 = 1;

/// Top-level `routes.yaml` document.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteConfig {
    pub version: u32,
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub routes: Vec<Route>,
}

/// Table-wide defaults; every field is overridable per route.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub strip_prefix: bool,
    #[serde(default)]
    pub websocket: bool,
    /// Operator-extensible deny-list of request headers, stripped from every
    /// upstream request in addition to the hardcoded gateway-reserved set.
    #[serde(default)]
    pub strip_request_headers: Vec<String>,
}

/// Default per-route upstream timeout when neither the route nor `defaults`
/// sets one.
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;

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

/// A single operator-defined route under `/api/`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub prefix: String,
    pub upstream: String,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub strip_prefix: Option<bool>,
    #[serde(default)]
    pub websocket: Option<bool>,
}

/// A route with its defaults folded in -- the shape the emitter consumes.
#[derive(Debug, Clone)]
pub struct ResolvedRoute {
    pub prefix: String,
    pub upstream: String,
    pub timeout_ms: u64,
    pub strip_prefix: bool,
    pub websocket: bool,
}

impl Route {
    /// Fold table defaults into this route.
    #[must_use]
    pub fn resolve(&self, defaults: &Defaults) -> ResolvedRoute {
        ResolvedRoute {
            prefix: self.prefix.clone(),
            upstream: self.upstream.clone(),
            timeout_ms: self.timeout_ms.unwrap_or(defaults.timeout_ms),
            strip_prefix: self.strip_prefix.unwrap_or(defaults.strip_prefix),
            websocket: self.websocket.unwrap_or(defaults.websocket),
        }
    }
}

impl RouteConfig {
    /// Parse a `routes.yaml` document.
    ///
    /// # Errors
    /// Returns the parse error if the YAML is malformed or violates the schema
    /// shape (unknown fields, wrong types).
    pub fn parse(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }

    /// Every route with defaults folded in, in document order.
    #[must_use]
    pub fn resolved_routes(&self) -> Vec<ResolvedRoute> {
        self.routes
            .iter()
            .map(|r| r.resolve(&self.defaults))
            .collect()
    }
}
