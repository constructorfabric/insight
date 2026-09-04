//! Golden-file snapshot tests for the emitted nginx.conf, plus a rejection case
//! for every validation rule (gateway DESIGN 3.8, 3.9; DD-GW-02).
//!
//! Regenerate the golden files after an intentional emitter change:
//! ```text
//! cargo run -p routegen -- --routes tools/routegen/tests/fixtures/full.routes.yaml \
//!   -o tools/routegen/tests/fixtures/full.nginx.conf
//! cargo run -p routegen -- --routes tools/routegen/tests/fixtures/stripprefix.routes.yaml \
//!   -o tools/routegen/tests/fixtures/stripprefix.nginx.conf
//! ```
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;

use routegen::{Settings, generate};

fn fixture(name: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("read fixture {}: {e}", p.display()))
}

fn assert_golden(routes: &str, conf: &str) {
    let got = generate(&fixture(routes), &Settings::default()).expect("generate should succeed");
    let expected = fixture(conf);
    assert_eq!(
        got, expected,
        "{conf} drifted from {routes}; regenerate it with routegen and commit the result"
    );
}

#[test]
fn golden_full() {
    // The canonical DESIGN 3.8 table exercising the full feature set: per-route
    // timeout, a websocket route with timeout_ms 0, an operator strip list, and
    // a shared upstream.
    assert_golden("full.routes.yaml", "full.nginx.conf");
}

#[test]
fn golden_strip_prefix() {
    // strip_prefix (default + per-route override), https upstream, and two
    // routes collapsing onto one deduplicated upstream block.
    assert_golden("stripprefix.routes.yaml", "stripprefix.nginx.conf");
}

#[test]
fn instance_token_route_bypasses_session_and_mcp_oauth() {
    let routes = "version: 1\nroutes:\n  - prefix: /api/sql/query\n    upstream: http://analytics:8086\n    auth: instance_token\n";
    let conf = generate(routes, &Settings::default()).unwrap();
    assert!(conf.contains("location = /api/sql/query {"));
    assert!(conf.contains("require(\"gateway\").pass_instance_token()"));
    assert!(!conf.contains("require(\"gateway\").pass_bearer()"));
    assert!(!conf.contains("require(\"gateway\").exchange()"));
    let sql_location = conf
        .split("location = /api/sql/query {")
        .nth(1)
        .unwrap()
        .split("        }\n")
        .next()
        .unwrap();
    for directive in [
        "proxy_set_header Host $host;",
        "proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;",
        "proxy_set_header X-Forwarded-Proto $scheme;",
        "proxy_connect_timeout 5s;",
        "proxy_read_timeout 30s;",
        "proxy_buffering off;",
    ] {
        assert!(sql_location.contains(directive), "missing: {directive}");
    }
    for path in ["/mcp", "/api/analytics", "/api/sql/query/extra"] {
        assert!(
            generate(
                &routes.replace("/api/sql/query", path),
                &Settings::default()
            )
            .is_err(),
            "path: {path}"
        );
    }
}

/// Every generated `/api/` location must carry the full hygiene block (DESIGN
/// 3.9): the Lua exchange, no browser Authorization survives, the session cookie
/// is stripped in Lua, a fresh correlation id, and gateway-authored forwarding.
#[test]
fn every_session_location_has_the_hygiene_block() {
    let conf = generate(&fixture("full.routes.yaml"), &Settings::default()).unwrap();
    // One access_by_lua per /api route (3 routes in the fixture).
    assert_eq!(
        conf.matches("access_by_lua_block { require(\"gateway\").exchange() }")
            .count(),
        3
    );
    // Unmatched /api and the internal surface both fail closed.
    assert!(conf.contains("location /api/ {\n            return 404;"));
    assert!(conf.contains("location /internal/ {\n            return 404;"));
    // Auth surface is a plain proxy (no exchange) and the SPA rides through.
    assert!(conf.contains("location /auth/ {"));
    // The SPA front is resolved lazily (variable proxy_pass + resolver) so the
    // gateway boots without a frontend present.
    assert!(conf.contains("location / {\n            set $insight_front \""));
    assert!(conf.contains("proxy_pass http://$insight_front;"));
    // HSTS on every response (G9).
    assert!(conf.contains("add_header Strict-Transport-Security"));
}

#[test]
fn real_ip_emitted_only_when_trusted_cidrs_configured() {
    let yaml = fixture("full.routes.yaml");
    // Default (no trusted proxies) -> no real_ip block, no baked network.
    let default = generate(&yaml, &Settings::default()).unwrap();
    assert!(!default.contains("set_real_ip_from"));
    assert!(!default.contains("real_ip_header"));
    // Configured -> the trust chain is emitted.
    let with_trust = generate(
        &yaml,
        &Settings {
            set_real_ip_from: vec!["10.0.0.0/8".into(), "192.168.0.0/16".into()],
            ..Settings::default()
        },
    )
    .unwrap();
    assert!(with_trust.contains("real_ip_header X-Forwarded-For;"));
    assert!(with_trust.contains("set_real_ip_from 10.0.0.0/8;"));
    assert!(with_trust.contains("set_real_ip_from 192.168.0.0/16;"));
}

#[test]
fn stub_status_is_loopback_only() {
    let conf = generate(&fixture("full.routes.yaml"), &Settings::default()).unwrap();
    let status_server = conf
        .split("server {")
        .find(|s| s.contains("stub_status;"))
        .expect("a server block serving stub_status");
    let listens: Vec<&str> = status_server
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("listen "))
        .collect();
    assert_eq!(listens, ["listen 127.0.0.1:8090;"]);
}

#[test]
fn mcp_oauth_metadata_and_jwks_are_fronted_by_the_gateway() {
    let conf = generate(&fixture("full.routes.yaml"), &Settings::default()).unwrap();
    assert!(conf.contains("location = /.well-known/oauth-authorization-server"));
    assert!(conf.contains("location = /.well-known/oauth-protected-resource/mcp"));
    assert!(conf.contains("location = /.well-known/jwks.json"));
    assert!(!conf.contains("location = /.well-known/openid-configuration"));
}

#[test]
fn configured_mcp_origin_reaches_bearer_challenges() {
    let conf = generate(
        &fixture("full.routes.yaml"),
        &Settings {
            mcp_public_url: Some("https://insight.example.com".to_owned()),
            ..Settings::default()
        },
    )
    .unwrap();
    assert!(conf.contains(
        "mcp_resource_metadata_url = \"https://insight.example.com/.well-known/oauth-protected-resource/mcp\""
    ));
}

#[test]
fn rejects_mcp_public_url_with_a_path() {
    let result = generate(
        &fixture("full.routes.yaml"),
        &Settings {
            mcp_public_url: Some("https://insight.example.com/base".to_owned()),
            ..Settings::default()
        },
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("MCP public URL must be an HTTP(S) origin")
    );
}

fn reject(yaml: &str) -> String {
    let err = generate(yaml, &Settings::default())
        .expect_err("expected validation to reject this config");
    format!("{err:#}")
}

#[test]
fn rejects_unknown_version() {
    let e = reject("version: 99\nroutes: []\n");
    assert!(e.contains("unsupported schema version 99"), "{e}");
}

#[test]
fn rejects_duplicate_prefix() {
    let e = reject(
        "version: 1\nroutes:\n\
         - {prefix: /api/a, upstream: 'http://h:1'}\n\
         - {prefix: /api/a, upstream: 'http://h:2'}\n",
    );
    assert!(e.contains("duplicate route prefix '/api/a'"), "{e}");
}

#[test]
fn rejects_prefix_outside_api() {
    let e = reject("version: 1\nroutes:\n- {prefix: /admin, upstream: 'http://h:1'}\n");
    assert!(e.contains("must start with '/api/'"), "{e}");
}

#[test]
fn rejects_upstream_without_host() {
    let e = reject("version: 1\nroutes:\n- {prefix: /api/a, upstream: 'http://:8080'}\n");
    assert!(e.contains("invalid upstream"), "{e}");
}

#[test]
fn rejects_upstream_with_bad_scheme() {
    let e = reject("version: 1\nroutes:\n- {prefix: /api/a, upstream: 'ftp://h:21'}\n");
    assert!(e.contains("scheme 'ftp' is not http/https"), "{e}");
}

#[test]
fn rejects_upstream_with_path() {
    let e = reject("version: 1\nroutes:\n- {prefix: /api/a, upstream: 'http://h:1/base'}\n");
    assert!(e.contains("must not carry a path"), "{e}");
}

#[test]
fn rejects_zero_timeout_without_websocket() {
    let e =
        reject("version: 1\nroutes:\n- {prefix: /api/a, upstream: 'http://h:1', timeout_ms: 0}\n");
    assert!(
        e.contains("timeout_ms 0 is only allowed with websocket: true"),
        "{e}"
    );
}

#[test]
fn allows_zero_timeout_with_websocket() {
    let ok = generate(
        "version: 1\nroutes:\n\
         - {prefix: /api/ws, upstream: 'http://h:1', timeout_ms: 0, websocket: true}\n",
        &Settings::default(),
    );
    assert!(ok.is_ok(), "{ok:?}");
}

#[test]
fn rejects_reserved_strip_headers() {
    for header in [
        "Authorization",
        "X-Correlation-Id",
        "X-Forwarded-For",
        "Cookie",
    ] {
        let e = reject(&format!(
            "version: 1\ndefaults:\n  strip_request_headers: [{header}]\nroutes: []\n"
        ));
        assert!(e.contains("gateway-reserved"), "{header}: {e}");
    }
}

#[test]
fn allows_x_tenant_id_in_strip_list() {
    // X-Tenant-ID is deliberately NOT reserved -- it is the tenant selector.
    let ok = generate(
        "version: 1\ndefaults:\n  strip_request_headers: [X-Tenant-ID]\nroutes: []\n",
        &Settings::default(),
    );
    assert!(ok.is_ok(), "{ok:?}");
}

#[test]
fn rejects_invalid_header_name() {
    let e =
        reject("version: 1\ndefaults:\n  strip_request_headers: [\"bad header\"]\nroutes: []\n");
    assert!(e.contains("not a valid HTTP header name"), "{e}");
}

#[test]
fn rejects_unknown_yaml_field() {
    let e = reject("version: 1\nbogus: true\nroutes: []\n");
    assert!(e.contains("parse routes.yaml"), "{e}");
}

#[test]
fn reports_all_violations_at_once() {
    // Two independent violations in one document -> both reported.
    let e = reject(
        "version: 1\nroutes:\n\
         - {prefix: /nope, upstream: 'http://h:1'}\n\
         - {prefix: /api/b, upstream: 'not-a-url'}\n",
    );
    assert!(e.contains("must start with '/api/'"), "{e}");
    assert!(e.contains("invalid upstream"), "{e}");
}
