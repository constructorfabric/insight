use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::error::MetricError;
use crate::infra::identity::{IdentityClient, IdentityProfile};

const SERVICE_SUBJECT_TYPE: &str = "service";

pub(crate) async fn authorize_person_ids(
    identity: &IdentityClient,
    ctx: &SecurityContext,
    authorization: Option<&str>,
    person_ids: &[Uuid],
) -> Result<(), CanonicalError> {
    if ctx.subject_type() == Some(SERVICE_SUBJECT_TYPE) {
        return Ok(());
    }

    authorize_visible_person_ids(identity, ctx.subject_id(), authorization, person_ids).await
}

pub(crate) async fn authorize_and_hydrate_person_profiles(
    identity: &IdentityClient,
    ctx: &SecurityContext,
    authorization: Option<&str>,
    person_ids: &[Uuid],
) -> Result<Vec<IdentityProfile>, CanonicalError> {
    if !identity.is_configured() {
        tracing::error!("identity service is not configured; reports cannot hydrate profiles");
        return Err(unavailable());
    }

    let caller = ctx.subject_id();
    if caller.is_nil() {
        tracing::error!("report access attempted with no resolved caller");
        return Err(unavailable());
    }

    let Some(authorization) = authorization else {
        tracing::error!("no Authorization header to forward for report profile hydration");
        return Err(unavailable());
    };

    let profiles = identity
        .profiles_batch(person_ids, Some(authorization))
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "report profile batch failed");
            unavailable()
        })?;

    if !profile_batch_contract_holds(person_ids, &profiles) {
        tracing::error!("identity profile batch violated its ordered-subset contract");
        return Err(unavailable());
    }

    let unmatched = person_ids.len().saturating_sub(profiles.len());
    if unmatched > 0 {
        tracing::warn!(
            unmatched,
            "report access denied: requested entities outside the caller's visible set"
        );
        return Err(MetricError::permission_denied()
            .with_reason("entity_not_visible")
            .create());
    }

    Ok(profiles)
}

fn profile_batch_contract_holds(requested: &[Uuid], profiles: &[IdentityProfile]) -> bool {
    let mut requested = requested.iter();
    profiles.iter().all(|profile| {
        if profile
            .supervisor
            .as_ref()
            .is_some_and(|supervisor| supervisor.person_id.is_nil())
        {
            return false;
        }

        requested
            .by_ref()
            .any(|person_id| *person_id == profile.person_id)
    })
}

async fn authorize_visible_person_ids(
    identity: &IdentityClient,
    caller: Uuid,
    authorization: Option<&str>,
    person_ids: &[Uuid],
) -> Result<(), CanonicalError> {
    if !identity.is_configured() {
        tracing::error!("identity service is not configured; person metrics cannot be authorized");
        return Err(unavailable());
    }

    if caller.is_nil() {
        tracing::error!("metric access attempted with no resolved caller");
        return Err(unavailable());
    }

    let Some(authorization) = authorization else {
        tracing::error!(caller = %caller, "no Authorization header to forward to identity");
        return Err(unavailable());
    };

    let visible = identity
        .visible_person_ids(person_ids, Some(authorization))
        .await
        .map_err(|e| {
            tracing::error!(error = %e, caller = %caller, "visibility check failed");
            unavailable()
        })?;

    // Only the count leaves this function: which ids were refused is not told
    // to the caller (an id they may not see is not theirs to learn about).
    let unmatched = person_ids
        .iter()
        .filter(|person_id| !visible.contains(*person_id))
        .count();
    if unmatched == 0 {
        return Ok(());
    }

    Err(denied(caller, unmatched))
}

// INVARIANT: the gate's only 403 — identity answered "not visible". Anything
// that stops the check from running uses `unavailable` instead.
fn denied(caller: Uuid, unmatched: usize) -> CanonicalError {
    tracing::warn!(
        caller = %caller,
        unmatched,
        "metric access denied: requested entities outside the caller's visible set"
    );
    MetricError::permission_denied()
        .with_reason("entity_not_visible")
        .create()
}

// `internal`, not `service_unavailable`: 500 is in the operation's declared
// response set.
fn unavailable() -> CanonicalError {
    CanonicalError::internal("metric access could not be authorized").create()
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test setup panics on a broken fixture"
)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Arc;

    use axum::Router;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::routing::post;

    use super::*;

    const CALLER: Uuid = Uuid::from_u128(0x018f_0000_0000_7000_8000_0000_0000_0001);
    const TENANT: Uuid = Uuid::from_u128(0x018f_0000_0000_7000_8000_0000_0000_0002);
    const SELF_PERSON: Uuid = Uuid::from_u128(0x018f_0000_0000_7000_8000_0000_0000_0011);
    const REPORT_PERSON: Uuid = Uuid::from_u128(0x018f_0000_0000_7000_8000_0000_0000_0012);
    const STRANGER_PERSON: Uuid = Uuid::from_u128(0x018f_0000_0000_7000_8000_0000_0000_0013);

    async fn spawn_identity(visible: &[Uuid]) -> IdentityClient {
        let visible = Arc::new(visible.iter().copied().collect::<HashSet<Uuid>>());

        let app = Router::new().route(
            "/v1/visible-persons",
            post(move |axum::Json(req): axum::Json<serde_json::Value>| {
                let visible = Arc::clone(&visible);
                async move {
                    let requested = req["person_ids"]
                        .as_array()
                        .map(|ids| {
                            ids.iter()
                                .filter_map(|v| v.as_str())
                                .filter_map(|v| Uuid::parse_str(v).ok())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let granted = requested
                        .into_iter()
                        .filter(|person_id| visible.contains(person_id))
                        .collect::<Vec<_>>();
                    axum::Json(serde_json::json!({"visible": granted}))
                }
            }),
        );

        IdentityClient::new(&serve(app).await).unwrap()
    }

    async fn spawn_failing_identity(status: StatusCode) -> IdentityClient {
        let app = Router::new().route(
            "/v1/visible-persons",
            post(move || async move { status.into_response() }),
        );
        IdentityClient::new(&serve(app).await).unwrap()
    }

    async fn spawn_profiles_identity(body: serde_json::Value) -> IdentityClient {
        let app = Router::new().route(
            "/v1/profiles/batch",
            post(move || {
                let body = body.clone();
                async move { axum::Json(body) }
            }),
        );
        IdentityClient::new(&serve(app).await).unwrap()
    }

    async fn serve(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn ctx_for(subject_type: &str, subject: Uuid) -> SecurityContext {
        SecurityContext::builder()
            .subject_id(subject)
            .subject_type(subject_type)
            .subject_tenant_id(TENANT)
            .build()
            .expect("subject and tenant are set")
    }

    fn status_of(result: Result<(), CanonicalError>) -> StatusCode {
        match result {
            Ok(()) => StatusCode::OK,
            Err(e) => e.into_response().status(),
        }
    }

    async fn authorize(
        identity: &IdentityClient,
        ctx: &SecurityContext,
        person_ids: &[Uuid],
    ) -> StatusCode {
        status_of(authorize_person_ids(identity, ctx, Some("Bearer tok"), person_ids).await)
    }

    #[tokio::test]
    async fn a_service_principal_bypasses_person_visibility() {
        let identity = spawn_identity(&[]).await;
        let ctx = ctx_for(SERVICE_SUBJECT_TYPE, CALLER);

        assert_eq!(
            status_of(authorize_person_ids(&identity, &ctx, None, &[STRANGER_PERSON]).await),
            StatusCode::OK,
        );
    }

    #[tokio::test]
    async fn ids_identity_reports_as_visible_are_admitted() {
        let identity = spawn_identity(&[SELF_PERSON, REPORT_PERSON]).await;
        let ctx = ctx_for("user", CALLER);

        assert_eq!(
            authorize(&identity, &ctx, &[SELF_PERSON, REPORT_PERSON]).await,
            StatusCode::OK,
        );
    }

    #[tokio::test]
    async fn person_outside_the_visible_set_is_forbidden() {
        let identity = spawn_identity(&[SELF_PERSON]).await;
        let ctx = ctx_for("user", CALLER);

        assert_eq!(
            authorize(&identity, &ctx, &[STRANGER_PERSON]).await,
            StatusCode::FORBIDDEN,
        );
    }

    #[tokio::test]
    async fn one_forbidden_id_rejects_the_whole_request() {
        let identity = spawn_identity(&[SELF_PERSON]).await;
        let ctx = ctx_for("user", CALLER);

        assert_eq!(
            authorize(&identity, &ctx, &[SELF_PERSON, STRANGER_PERSON]).await,
            StatusCode::FORBIDDEN,
        );
    }

    #[tokio::test]
    async fn identity_outage_is_a_server_error_not_forbidden() {
        let identity = spawn_failing_identity(StatusCode::INTERNAL_SERVER_ERROR).await;
        let ctx = ctx_for("user", CALLER);

        assert_eq!(
            authorize(&identity, &ctx, &[SELF_PERSON]).await,
            StatusCode::INTERNAL_SERVER_ERROR,
            "a dependency outage must not read as a denial",
        );
    }

    #[tokio::test]
    async fn an_identity_without_the_endpoint_is_a_server_error_not_forbidden() {
        for status in [StatusCode::NOT_FOUND, StatusCode::METHOD_NOT_ALLOWED] {
            let identity = spawn_failing_identity(status).await;
            let ctx = ctx_for("user", CALLER);

            assert_eq!(
                authorize(&identity, &ctx, &[SELF_PERSON]).await,
                StatusCode::INTERNAL_SERVER_ERROR,
                "identity answering {status} must not read as a denial",
            );
        }
    }

    #[tokio::test]
    async fn unconfigured_identity_refuses_person_access() {
        let identity = IdentityClient::new("").unwrap();
        let ctx = ctx_for("user", CALLER);

        assert_eq!(
            authorize(&identity, &ctx, &[SELF_PERSON]).await,
            StatusCode::INTERNAL_SERVER_ERROR,
            "without an authorization backend the gate fails closed",
        );
    }

    #[tokio::test]
    async fn service_subject_is_not_gated() {
        let identity = IdentityClient::new("").unwrap();
        let ctx = ctx_for("service", CALLER);

        assert_eq!(
            authorize(&identity, &ctx, &[STRANGER_PERSON]).await,
            StatusCode::OK,
        );
    }

    #[tokio::test]
    async fn anonymous_caller_is_a_server_error_not_forbidden() {
        let identity = spawn_identity(&[SELF_PERSON]).await;
        let ctx = ctx_for("user", Uuid::nil());

        assert_eq!(
            authorize(&identity, &ctx, &[SELF_PERSON]).await,
            StatusCode::INTERNAL_SERVER_ERROR,
            "an unresolved caller is a broken authn path, not a denial"
        );
    }

    #[tokio::test]
    async fn a_request_without_a_bearer_to_forward_is_a_server_error_not_forbidden() {
        let identity = spawn_identity(&[SELF_PERSON]).await;
        let ctx = ctx_for("user", CALLER);

        let status = status_of(authorize_person_ids(&identity, &ctx, None, &[SELF_PERSON]).await);
        assert_eq!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "a broken authn path is not a visibility denial"
        );
    }

    #[tokio::test]
    async fn profile_hydration_requires_every_requested_person_in_order() {
        let identity = spawn_profiles_identity(serde_json::json!({
            "profiles": [
                {"person_id": SELF_PERSON, "attributes": {}},
                {"person_id": REPORT_PERSON, "attributes": {"display_name": "Example User"}}
            ]
        }))
        .await;
        let ctx = ctx_for("user", CALLER);

        let profiles = authorize_and_hydrate_person_profiles(
            &identity,
            &ctx,
            Some("Bearer tok"),
            &[SELF_PERSON, REPORT_PERSON],
        )
        .await
        .unwrap();

        assert_eq!(
            profiles
                .iter()
                .map(|profile| profile.person_id)
                .collect::<Vec<_>>(),
            vec![SELF_PERSON, REPORT_PERSON]
        );
    }

    #[tokio::test]
    async fn omitted_profile_rejects_the_whole_report_as_not_visible() {
        let identity = spawn_profiles_identity(serde_json::json!({
            "profiles": [{"person_id": SELF_PERSON, "attributes": {}}]
        }))
        .await;
        let ctx = ctx_for("user", CALLER);

        let error = authorize_and_hydrate_person_profiles(
            &identity,
            &ctx,
            Some("Bearer tok"),
            &[SELF_PERSON, STRANGER_PERSON],
        )
        .await
        .unwrap_err();

        assert_eq!(error.into_response().status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn reordered_profile_success_is_a_dependency_error() {
        let identity = spawn_profiles_identity(serde_json::json!({
            "profiles": [
                {"person_id": REPORT_PERSON, "attributes": {}},
                {"person_id": SELF_PERSON, "attributes": {}}
            ]
        }))
        .await;
        let ctx = ctx_for("user", CALLER);

        let error = authorize_and_hydrate_person_profiles(
            &identity,
            &ctx,
            Some("Bearer tok"),
            &[SELF_PERSON, REPORT_PERSON],
        )
        .await
        .unwrap_err();

        assert_eq!(
            error.into_response().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn unrequested_profile_success_is_a_dependency_error() {
        let identity = spawn_profiles_identity(serde_json::json!({
            "profiles": [{"person_id": STRANGER_PERSON, "attributes": {}}]
        }))
        .await;
        let ctx = ctx_for("user", CALLER);

        let error = authorize_and_hydrate_person_profiles(
            &identity,
            &ctx,
            Some("Bearer tok"),
            &[SELF_PERSON],
        )
        .await
        .unwrap_err();

        assert_eq!(
            error.into_response().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn nil_supervisor_success_is_a_dependency_error() {
        let identity = spawn_profiles_identity(serde_json::json!({
            "profiles": [{
                "person_id": SELF_PERSON,
                "attributes": {},
                "supervisor": {"person_id": Uuid::nil(), "attributes": {}}
            }]
        }))
        .await;
        let ctx = ctx_for("user", CALLER);

        let error = authorize_and_hydrate_person_profiles(
            &identity,
            &ctx,
            Some("Bearer tok"),
            &[SELF_PERSON],
        )
        .await
        .unwrap_err();

        assert_eq!(
            error.into_response().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
