//! Covers the env-driven entrypoint (`Config::from_env` + `run`) and the
//! `FAKEIDP_DEV_USER_EMAIL` override by actually booting the server the way the
//! binary does. Kept in its own test file so it runs as a separate process and
//! never races `flow.rs` on `std::env`.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;

const DEV_EMAIL: &str = "wizard-dev@example.test";

#[tokio::test]
async fn boots_from_env_and_honors_dev_user_override() {
    // Grab a free port, then point the env at it so `run()` binds somewhere
    // predictable (and we avoid colliding with a real service on 8084).
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let issuer = format!("http://127.0.0.1:{port}");

    // SAFETY: set before any other thread reads the environment; this is the
    // only test in this process and it mutates env once, up front.
    unsafe {
        std::env::set_var("FAKEIDP_BIND", format!("127.0.0.1:{port}"));
        std::env::set_var("FAKEIDP_ISSUER", &issuer);
        std::env::set_var("FAKEIDP_TOKEN_TTL", "123");
        std::env::set_var("FAKEIDP_DEFAULT_AUD", "authenticator");
        std::env::set_var("FAKEIDP_DEV_USER_EMAIL", DEV_EMAIL);
    }

    tokio::spawn(fakeidp::run());

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    // Wait for the server (started via run()) to come up.
    let mut booted = false;
    for _ in 0..100 {
        if let Ok(resp) = client
            .get(format!("{issuer}/.well-known/openid-configuration"))
            .send()
            .await
            && resp.status().is_success()
        {
            let body: Value = resp.json().await.unwrap();
            assert_eq!(body["issuer"], issuer, "issuer comes from FAKEIDP_ISSUER");
            booted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(booted, "fakeidp did not come up via run()");

    // Default login (no `user=`) → the first user, whose email must be the
    // FAKEIDP_DEV_USER_EMAIL override rather than the baked default.
    let authz = client
        .get(format!("{issuer}/authorize"))
        .query(&[("redirect_uri", "http://rp.test/cb")])
        .send()
        .await
        .unwrap();
    let location = authz.headers()["location"].to_str().unwrap();
    let code = location
        .split_once('?')
        .unwrap()
        .1
        .split('&')
        .find_map(|kv| kv.strip_prefix("code="))
        .unwrap()
        .to_string();
    let tok: Value = client
        .post(format!("{issuer}/token"))
        .form(&[("grant_type", "authorization_code"), ("code", &code)])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let payload = tok["id_token"].as_str().unwrap().split('.').nth(1).unwrap();
    let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
    assert_eq!(
        claims["email"], DEV_EMAIL,
        "dev user email override applied"
    );
    assert_eq!(tok["expires_in"], 123, "token_ttl comes from env");
}
