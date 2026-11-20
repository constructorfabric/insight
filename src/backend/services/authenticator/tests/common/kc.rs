use std::process::Command;

fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

pub fn base() -> String {
    env("E2E_KC_BASE", "http://localhost:8084")
}

pub fn realm() -> String {
    env("E2E_KC_REALM", "insight")
}

pub fn user_password() -> String {
    env("E2E_USER_PASSWORD", "insight-dev")
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("reqwest client must build")
}

pub async fn authorize(authorize_url: &str, user: &str) -> String {
    let http = http();

    let page = http.get(authorize_url).send().await.unwrap();
    assert_eq!(
        page.status(),
        200,
        "the authorize endpoint must serve the login form"
    );
    let auth_cookies: Vec<String> = page
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .filter_map(|raw| raw.split(';').next())
        .map(str::to_owned)
        .collect();
    let body = page.text().await.unwrap();
    let form_action = login_form_action(&body);

    let submitted = http
        .post(&form_action)
        .header(reqwest::header::COOKIE, auth_cookies.join("; "))
        .form(&[
            ("username", user),
            ("password", &user_password()),
            ("credentialId", ""),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(
        submitted.status(),
        302,
        "the credential POST for {user} must redirect to the RP; page said: {}",
        submitted
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(500)
            .collect::<String>()
    );
    submitted.headers()[reqwest::header::LOCATION]
        .to_str()
        .unwrap()
        .to_owned()
}

pub async fn login(http: &super::Client, auth_base: &str, user: &str) -> String {
    let login = http
        .get(format!("{auth_base}/auth/login"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        login.status(),
        302,
        "GET /auth/login must redirect to the IdP"
    );
    let authorize_url = login.headers()[reqwest::header::LOCATION]
        .to_str()
        .unwrap()
        .to_owned();

    let callback = authorize(&authorize_url, user).await;

    let cb = http.get(&callback).send().await.unwrap();
    assert_eq!(
        cb.status(),
        302,
        "callback must set the cookie and redirect"
    );
    session_cookie(&cb).expect("callback must set __Host-sid")
}

pub fn session_cookie(resp: &reqwest::Response) -> Option<String> {
    for hv in resp.headers().get_all(reqwest::header::SET_COOKIE) {
        let raw = hv.to_str().ok()?;
        for part in raw.split(';') {
            if let Some(v) = part.trim().strip_prefix("__Host-sid=")
                && !v.is_empty()
            {
                return Some(v.to_owned());
            }
        }
    }
    None
}

fn login_form_action(page: &str) -> String {
    let from = page.find("id=\"kc-form-login\"").unwrap_or(0);
    let action_at = page[from..]
        .find("action=\"")
        .expect("the authorize page must contain the login form action")
        + from
        + "action=\"".len();
    let action = &page[action_at
        ..action_at
            + page[action_at..]
                .find('"')
                .expect("unterminated action attribute")];
    action.replace("&amp;", "&")
}

async fn admin_token(http: &reqwest::Client) -> String {
    let resp = http
        .post(format!(
            "{}/realms/master/protocol/openid-connect/token",
            base()
        ))
        .form(&[
            ("grant_type", "password"),
            ("client_id", "admin-cli"),
            ("username", &env("E2E_KC_ADMIN_USER", "admin")),
            ("password", &env("E2E_KC_ADMIN_PASSWORD", "admin")),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "the Keycloak admin token grant must succeed"
    );
    resp.json::<serde_json::Value>().await.unwrap()["access_token"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn user_representation(
    http: &reqwest::Client,
    token: &str,
    email: &str,
) -> serde_json::Value {
    let resp = http
        .get(format!("{}/admin/realms/{}/users", base(), realm()))
        .query(&[("email", email), ("exact", "true")])
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "the admin user lookup must succeed");
    let mut users: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(
        users.len(),
        1,
        "exactly one realm user expected for {email}"
    );
    users.remove(0)
}

pub async fn logout_user(email: &str) {
    let http = http();
    let token = admin_token(&http).await;
    let user = user_representation(&http, &token, email).await;
    let id = user["id"].as_str().unwrap();
    let resp = http
        .post(format!(
            "{}/admin/realms/{}/users/{id}/logout",
            base(),
            realm()
        ))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "the admin logout for {email} must succeed, got {}",
        resp.status()
    );
}

pub async fn set_user_enabled(email: &str, enabled: bool) {
    let http = http();
    let token = admin_token(&http).await;
    let mut user = user_representation(&http, &token, email).await;
    let id = user["id"].as_str().unwrap().to_owned();
    user["enabled"] = serde_json::Value::Bool(enabled);
    let resp = http
        .put(format!("{}/admin/realms/{}/users/{id}", base(), realm()))
        .bearer_auth(&token)
        .json(&user)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "setting enabled={enabled} for {email} must succeed, got {}",
        resp.status()
    );
}

pub fn idp_outage() -> IdpOutage {
    let container = env("E2E_KC_CONTAINER", "authenticator-e2e-keycloak");
    let status = Command::new("docker")
        .args(["pause", &container])
        .status()
        .expect("docker must be runnable");
    assert!(status.success(), "docker pause {container} must succeed");
    IdpOutage { container }
}

pub struct IdpOutage {
    container: String,
}

impl Drop for IdpOutage {
    fn drop(&mut self) {
        // Best effort only: this runs during unwinding, where a panic aborts.
        let thawed = Command::new("docker")
            .args(["unpause", &self.container])
            .status()
            .is_ok_and(|status| status.success());
        assert!(
            thawed || std::thread::panicking(),
            "docker unpause {} must succeed",
            self.container
        );
    }
}
