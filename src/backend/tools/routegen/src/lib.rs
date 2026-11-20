#![expect(
    clippy::missing_errors_doc,
    reason = "lib.rs only serves this crate's binary and tests; service and tool crates carry no doc comments"
)]

pub mod emit;
pub mod schema;
pub mod validate;

pub use emit::Settings;
pub use schema::RouteConfig;
pub use validate::ValidationErrors;

pub fn generate(yaml: &str, settings: &Settings) -> anyhow::Result<String> {
    let config = RouteConfig::parse(yaml).map_err(|e| anyhow::anyhow!("parse routes.yaml: {e}"))?;
    validate::validate(&config)?;
    emit::emit(&config, settings)
}
