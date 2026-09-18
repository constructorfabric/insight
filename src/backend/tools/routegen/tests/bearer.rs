//! Bearer-authenticated routes: which prefixes may carry one, and the
//! challenge each emits.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use routegen::{RouteConfig, Settings, generate};

const ORIGIN: &str = "https://insight.example.invalid";

fn settings_with_origin() -> Settings {
    Settings {
        mcp_public_url: Some(ORIGIN.to_owned()),
        ..Settings::default()
    }
}

fn two_mcp_routes() -> &'static str {
    r#"
version: 1
routes:
  - prefix: /mcp
    upstream: http://analytics:8086
    auth: bearer
  - prefix: /mcp/v3
    upstream: http://insight-v3-core:8087
    auth: bearer
    mcp_scope: "mcp:author"
"#
}

#[test]
fn each_bearer_route_challenges_with_its_own_metadata_document_and_scope() {
    let conf = generate(two_mcp_routes(), &settings_with_origin()).expect("the document is valid");

    assert!(
        conf.contains(&format!(
            r#"pass_bearer("{ORIGIN}/.well-known/oauth-protected-resource/mcp", "mcp:query")"#
        )),
        "{conf}"
    );
    assert!(
        conf.contains(&format!(
            r#"pass_bearer("{ORIGIN}/.well-known/oauth-protected-resource/mcp/v3", "mcp:author")"#
        )),
        "{conf}"
    );
}

#[test]
fn each_bearer_route_gets_a_metadata_document_location_of_its_own() {
    let conf = generate(two_mcp_routes(), &settings_with_origin()).expect("the document is valid");

    assert!(
        conf.contains("location = /.well-known/oauth-protected-resource/mcp {"),
        "{conf}"
    );
    assert!(
        conf.contains("location = /.well-known/oauth-protected-resource/mcp/v3 {"),
        "{conf}"
    );
}

#[test]
fn a_bearer_route_with_no_declared_scope_keeps_the_read_only_one() {
    let yaml = r"
version: 1
routes:
  - prefix: /mcp
    upstream: http://analytics:8086
    auth: bearer
";

    let config = RouteConfig::parse(yaml).expect("the document is valid");

    assert_eq!(config.resolved_routes()[0].mcp_scope, "mcp:query");
}

#[test]
fn a_deployment_that_has_not_published_an_origin_emits_no_metadata_url() {
    let yaml = r"
version: 1
routes:
  - prefix: /mcp
    upstream: http://analytics:8086
    auth: bearer
";

    let conf = generate(yaml, &Settings::default()).expect("the document is valid");

    assert!(conf.contains(r#"pass_bearer(nil, "mcp:query")"#), "{conf}");
}

#[test]
fn a_bearer_route_on_an_undeclared_prefix_is_refused() {
    let yaml = r"
version: 1
routes:
  - prefix: /mcp/v4
    upstream: http://somewhere:8087
    auth: bearer
";

    let error = generate(yaml, &settings_with_origin())
        .expect_err("only the declared MCP paths may carry a bearer route");

    assert!(error.to_string().contains("/mcp/v4"), "{error}");
}

#[test]
fn a_session_route_may_not_borrow_an_mcp_prefix() {
    let yaml = r"
version: 1
routes:
  - prefix: /mcp/v3
    upstream: http://insight-v3-core:8087
";

    assert!(
        generate(yaml, &settings_with_origin()).is_err(),
        "a session route must live under /api/"
    );
}
