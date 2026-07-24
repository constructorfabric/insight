//! End-to-end OIDC back-channel logout against a running authenticator +
//! fakeidp + Redis (nginx+auth step 10, item 3).
//!
//! `#[ignore]` by default (needs the stack up with
//! `FAKEIDP_BACKCHANNEL_URL` pointing at the authenticator; `run-e2e.sh`
//! wires it):
//!
//! ```text
//! AUTH_BASE=http://localhost:8083 FAKEIDP_PUBLIC=http://localhost:8084 \
//!   cargo test -p authenticator --test e2e_backchannel -- --ignored --nocapture
//! ```
//!
//! Drives fakeidp's `POST /_control/backchannel/{email}` hook: the fake IdP
//! fires a signed `logout_token` at the authenticator, and every session of
//! that user dies through the standard revoke pipeline.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::doc_markdown)]

const COOKIE: &str = "__Host-sid";

fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

fn rewrite_host(url: &str) -> String {
    match (
        std::env::var("FAKEIDP_REWRITE_FROM"),
        std::env::var("FAKEIDP_REWRITE_TO"),
    ) {
        (Ok(from), Ok(to)) if !from.is_empty() => url.replace(&from, &to),
        _ => url.to_owned(),
    }
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

async fn login(http: &reqwest::Client, auth_base: &str, user: &str) -> String {
    let login = http
        .get(format!("{auth_base}/auth/login"))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), 302);
    let authorize = rewrite_host(login.headers()[reqwest::header::LOCATION].to_str().unwrap());
    let sep = if authorize.contains('?') { '&' } else { '?' };
    let authorized = http
        .get(format!("{authorize}{sep}user={user}"))
        .send()
        .await
        .unwrap();
    assert_eq!(authorized.status(), 302);
    let callback = rewrite_host(
        authorized.headers()[reqwest::header::LOCATION]
            .to_str()
            .unwrap(),
    );
    let cb = http.get(&callback).send().await.unwrap();
    assert_eq!(cb.status(), 302);
    cookie_from(&cb).expect("callback must set __Host-sid")
}

async fn authz_status(http: &reqwest::Client, auth_base: &str, token: &str) -> u16 {
    http.get(format!("{auth_base}/internal/authz"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={token}"))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

#[tokio::test]
#[ignore = "requires a running authenticator + fakeidp + Redis stack"]
async fn back_channel_logout_kills_the_users_sessions() {
    let auth_base = env("AUTH_BASE", "http://localhost:8083");
    let idp_public = env("FAKEIDP_PUBLIC", "http://localhost:8084");
    let test_user = env("E2E_USER", "dev@company.nonpresent");
    let http = client();

    // Two devices, both alive.
    let token_a = login(&http, &auth_base, &test_user).await;
    let token_b = login(&http, &auth_base, &test_user).await;
    assert_eq!(authz_status(&http, &auth_base, &token_a).await, 200);
    assert_eq!(authz_status(&http, &auth_base, &token_b).await, 200);

    // The IdP fires a back-channel logout_token at the authenticator.
    let fired = http
        .post(format!("{idp_public}/_control/backchannel/{test_user}"))
        .send()
        .await
        .unwrap();
    assert_eq!(fired.status(), 200, "fakeidp control hook must succeed");
    let body: serde_json::Value = fired.json().await.unwrap();
    assert_eq!(
        body["rp_status"], 200,
        "the authenticator must answer the logout_token with 200, got {body}"
    );

    // fakeidp's users have ONE OIDC sid per user (users.yaml), so the sid-index
    // path revokes every session created under it: both devices die.
    assert_eq!(
        authz_status(&http, &auth_base, &token_a).await,
        401,
        "device A must be logged out by back-channel logout"
    );
    assert_eq!(
        authz_status(&http, &auth_base, &token_b).await,
        401,
        "device B must be logged out by back-channel logout"
    );

    // A rejected (unsigned garbage) token is a 400, not a revoke.
    let bad = http
        .post(format!("{auth_base}/auth/oidc/back-channel-logout"))
        .form(&[("logout_token", "garbage.token.value")])
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400, "a malformed logout_token must be 400");
}
