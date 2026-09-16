use super::*;

#[test]
fn the_admin_role_id_matches_the_identity_services_own_constant() {
    // identity-resolution's roles_repo::ADMIN_ROLE_ID, and the id the
    // front end gates on. A mismatch would deny every caller.
    assert_eq!(
        ADMIN_ROLE_ID.to_string(),
        "a4d11000-0000-4000-8000-000000000001"
    );
}

#[test]
fn a_trailing_slash_does_not_double_up_in_the_path() {
    let client = IdentityClient::new("http://identity-resolution:8082/")
        .unwrap_or_else(|error| panic!("the client builds: {error}"));

    let Mode::Live { base_url, .. } = &client.mode else {
        panic!("a configured client is live");
    };
    assert_eq!(base_url, "http://identity-resolution:8082");
    assert!(client.is_configured());
}

#[test]
fn identity_refusing_the_caller_is_told_apart_from_identity_failing() {
    assert!(IdentityError::Refused(401).is_about_the_caller());
    assert!(IdentityError::Refused(403).is_about_the_caller());
    // A broken identity service is our problem, not the caller's.
    assert!(!IdentityError::Refused(500).is_about_the_caller());
    assert!(!IdentityError::Refused(502).is_about_the_caller());
}

#[test]
fn an_unset_url_is_not_configured() {
    let client =
        IdentityClient::new("").unwrap_or_else(|error| panic!("the client builds: {error}"));

    assert!(!client.is_configured());
}

#[test]
fn the_roles_a_caller_holds_decide() {
    let admin: MeResponse = serde_json::from_value(serde_json::json!({
        "roles": [{ "role_id": "a4d11000-0000-4000-8000-000000000001" }]
    }))
    .unwrap_or_else(|error| panic!("the fixture parses: {error}"));
    let other: MeResponse = serde_json::from_value(serde_json::json!({
        "roles": [{ "role_id": "00000000-0000-4000-8000-000000000009" }]
    }))
    .unwrap_or_else(|error| panic!("the fixture parses: {error}"));

    assert!(admin.roles.iter().any(|role| role.role_id == ADMIN_ROLE_ID));
    assert!(!other.roles.iter().any(|role| role.role_id == ADMIN_ROLE_ID));
}
