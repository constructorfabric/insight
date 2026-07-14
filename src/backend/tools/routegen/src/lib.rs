//! routegen -- the gateway route configurator (gateway DESIGN DD-GW-02).
//!
//! `routes.yaml` in, a complete validated `nginx.conf` out. Humans never write a
//! `location` block; every generated `/api/` location carries the full auth +
//! hygiene block by construction (DESIGN 3.9), closing the header-hygiene and
//! auth-bypass risks the deleted Router spec carried.

pub mod emit;
pub mod schema;
pub mod validate;

pub use emit::Settings;
pub use schema::RouteConfig;
pub use validate::ValidationErrors;

/// Parse, validate, and compile a `routes.yaml` document into `nginx.conf`.
///
/// # Errors
/// Fails on malformed YAML, any semantic validation violation (see
/// [`validate::validate`]), or an unparseable settings/upstream URL.
pub fn generate(yaml: &str, settings: &Settings) -> anyhow::Result<String> {
    let config = RouteConfig::parse(yaml).map_err(|e| anyhow::anyhow!("parse routes.yaml: {e}"))?;
    validate::validate(&config)?;
    emit::emit(&config, settings)
}
