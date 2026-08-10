//! End-to-end host-keyed issuer selection (ADR-0003) against an authenticator
//! configured with a two-entry `idp.hosts` map in front of two realms of the
//! one Keycloak, plus the flat-config instance for the degenerate case.
//!
//! `#[ignore]` by default (needs the stack up); run-e2e.sh wires it:
//!
//! ```text
//! AUTH_BASE=http://localhost:8087 AUTH_FLAT_BASE=http://localhost:8083 \
//!   AUTH3_LOG=/tmp/authenticator-e2e-auth3.log \
//!   E2E_IDP_ISSUER=http://localhost:8084/realms/insight \
//!   E2E_IDP2_ISSUER=http://localhost:8084/realms/insight-b \
//!   cargo test -p authenticator --test e2e_hostmap -- --ignored --nocapture
//! ```
//!
//! Covers: Host routing at `/auth/login`, unknown-host 403 + audit line,
//! callback under a different Host, and the flat-config degenerate case.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

const COOKIE: &str = "__Host-sid";
const HOST_A: &str = "tenant-a.example";
const HOST_B: &str = "tenant-b.example";

fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn location_of(resp: &reqwest::Response) -> String {
    resp.headers()[reqwest::header::LOCATION]
        .to_str()
        .unwrap()
        .to_owned()
}

fn cookie_from(resp: &reqwest::Response) -> Option<String> {
    for hv in resp.headers().get_all(reqwest::header::SET_COOKIE) {
        let raw = hv.to_str().ok()?;
        for part in raw.split(';') {
            if let Some(v) = part.trim().strip_prefix(&format!("{COOKIE}="))
                && !v.is_empty()
            {
                return Some(v.to_owned());
            }
        }
    }
    None
}

#[tokio::test]
#[ignore = "requires the run-e2e.sh stack (two realms + hosts-map authenticator)"]
async fn host_selects_the_issuer_and_the_callback_stays_pinned() {
    let auth_base = env("AUTH_BASE", "http://localhost:8087");
    let idp_a = env("E2E_IDP_ISSUER", "http://localhost:8084/realms/insight");
    let idp_b = env("E2E_IDP2_ISSUER", "http://localhost:8084/realms/insight-b");
    let test_user = env("E2E_USER", "dev@company.nonpresent");
    let http = common::client();

    // Each configured host is routed to its own issuer's authorize endpoint.
    for (host, idp) in [(HOST_A, &idp_a), (HOST_B, &idp_b)] {
        let login = http
            .get(format!("{auth_base}/auth/login"))
            .header("Host", host)
            .send()
            .await
            .unwrap();
        assert_eq!(login.status(), 302, "login via {host} should redirect");
        let authorize = location_of(&login);
        assert!(
            authorize.starts_with(idp.as_str()),
            "Host {host} must select issuer {idp}, got {authorize}"
        );
    }

    // Full loop through tenant-a; the callback is presented with tenant-b's
    // Host to prove the issuer pinned in the login state wins over any Host
    // ambiguity after login.
    let login = http
        .get(format!("{auth_base}/auth/login?return_to=/dashboard"))
        .header("Host", HOST_A)
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), 302);
    let authorize = location_of(&login);
    let callback = common::kc::authorize(&authorize, &test_user).await;

    let cb = http
        .get(&callback)
        .header("Host", HOST_B)
        .send()
        .await
        .unwrap();
    assert_eq!(
        cb.status(),
        302,
        "callback must complete against the login-pinned issuer regardless of Host"
    );
    assert_eq!(location_of(&cb), "/dashboard");
    let token = cookie_from(&cb).expect("callback must set the session cookie");

    let authz = http
        .get(format!("{auth_base}/internal/authz"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(authz.status(), 200, "the minted session must exchange");
}

#[tokio::test]
#[ignore = "requires the run-e2e.sh stack (two realms + hosts-map authenticator)"]
async fn unknown_host_is_rejected_fail_closed_with_an_audit_event() {
    let auth_base = env("AUTH_BASE", "http://localhost:8087");
    let http = common::client();

    let denied = http
        .get(format!("{auth_base}/auth/login"))
        .header("Host", "unknown.example")
        .send()
        .await
        .unwrap();
    assert_eq!(
        denied.status(),
        403,
        "a Host matching no configured issuer must be rejected fail-closed"
    );
    let body: serde_json::Value = denied.json().await.unwrap();
    assert_eq!(body["status"], 403, "problem+json body expected: {body}");

    // The denial is audited (`event = "login_denied_unknown_host"`).
    let log_path = env("AUTH3_LOG", "/tmp/authenticator-e2e-auth3.log");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let log = std::fs::read_to_string(&log_path)
        .unwrap_or_else(|e| panic!("read authenticator log {log_path}: {e}"));
    assert!(
        log.contains("login_denied_unknown_host"),
        "the unknown-host denial must emit its audit event (log: {log_path})"
    );
}

#[tokio::test]
#[ignore = "requires the run-e2e.sh stack (two realms + hosts-map authenticator)"]
async fn flat_single_issuer_config_matches_every_host() {
    let flat_base = env("AUTH_FLAT_BASE", "http://localhost:8083");
    let idp_a = env("E2E_IDP_ISSUER", "http://localhost:8084/realms/insight");
    let http = common::client();

    // The degenerate (map-less) instance serves any Host, exactly as before.
    for host in ["whatever.example", "another.example:8443"] {
        let login = http
            .get(format!("{flat_base}/auth/login"))
            .header("Host", host)
            .send()
            .await
            .unwrap();
        assert_eq!(login.status(), 302, "flat config must serve Host {host}");
        assert!(
            location_of(&login).starts_with(idp_a.as_str()),
            "flat config always routes to its one issuer"
        );
    }
}
