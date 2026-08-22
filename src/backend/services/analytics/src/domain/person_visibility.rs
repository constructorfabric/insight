use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::error::MetricError;
use crate::infra::identity::IdentityClient;

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
}
