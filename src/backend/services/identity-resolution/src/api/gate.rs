//! Shared admin gate for the mutating / admin identity-resolution endpoints.
//!
//! Ported from the .NET `CallerAdminCheck`: the caller is the gateway-JWT
//! subject (`SecurityContext::subject_id`, verified by the host authn pipeline —
//! `NGINX_BFF` R1), which must hold an active `admin` role in the tenant. Reused
//! by the persons-seed, roles, person-roles, and visibility endpoints.

use sea_orm::DatabaseConnection;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::error::AccessError;
use crate::infra::db::roles_repo;

/// Require an identified caller (gateway-JWT subject). Returns the caller
/// `person_id`, or 401 when the JWT carries no person subject. The baseline gate
/// for authenticated-but-not-admin endpoints (e.g. subchart).
///
/// # Errors
///
/// 401 if the JWT carries no person subject.
pub(crate) fn require_caller(ctx: &SecurityContext) -> Result<Uuid, CanonicalError> {
    let caller = ctx.subject_id();
    if caller.is_nil() {
        return Err(CanonicalError::unauthenticated()
            .with_reason("caller not identified: the gateway JWT carries no person subject")
            .create());
    }
    // A nil tenant reaches here only from a misconfigured token (the gateway JWT
    // should always carry a real tenant). Surface it as an explicit 400 instead
    // of silently flowing a nil tenant into every query (→ empty/404), which is
    // hard to diagnose. Rough parity with the .NET `tenant_unresolved` 400.
    if ctx.subject_tenant_id().is_nil() {
        return Err(AccessError::failed_precondition()
            .with_precondition_violation(
                "tenant",
                "tenant not resolved from the gateway JWT",
                "tenant_unresolved",
            )
            .create());
    }
    Ok(caller)
}

/// Resolve the caller (gateway-JWT subject) and require an active `admin` role
/// in the tenant. Returns the caller `person_id`, or 401 (no subject) / 403
/// (not admin).
///
/// # Errors
///
/// 401 if the JWT carries no person subject, 403 if the caller is not an admin,
/// 500 on DB error.
pub(crate) async fn require_admin(
    db: &DatabaseConnection,
    ctx: &SecurityContext,
) -> Result<Uuid, CanonicalError> {
    let caller = require_caller(ctx)?;
    let is_admin = roles_repo::has_active_admin(db, ctx.subject_tenant_id(), caller)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "admin role check failed");
            CanonicalError::internal("failed to verify caller permissions").create()
        })?;
    if !is_admin {
        tracing::warn!(
            caller = %caller,
            tenant = %ctx.subject_tenant_id(),
            "admin gate denied: caller has no active admin role"
        );
        return Err(AccessError::permission_denied()
            .with_reason("admin role required for this operation")
            .create());
    }
    Ok(caller)
}

/// Require a SERVICE principal (gateway JWT `sub_type=service`). Used by the
/// internal S2S endpoints that run before a tenant/caller identity exists (the
/// login-bootstrap by-email lookup). 403 for any non-service caller.
///
/// # Errors
///
/// 403 if the caller is not a service principal.
pub(crate) fn require_service(ctx: &SecurityContext) -> Result<(), CanonicalError> {
    if ctx.subject_type() != Some("service") {
        return Err(AccessError::permission_denied()
            .with_reason("this endpoint is restricted to service principals (sub_type=service)")
            .create());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(subject_type: &str) -> anyhow::Result<SecurityContext> {
        SecurityContext::builder()
            .subject_id(Uuid::from_u128(1))
            .subject_type(subject_type)
            .subject_tenant_id(Uuid::from_u128(2))
            .build()
            .map_err(|e| anyhow::anyhow!("build security context: {e:?}"))
    }

    #[test]
    fn require_service_allows_only_service_principals() -> anyhow::Result<()> {
        assert!(require_service(&ctx("service")?).is_ok(), "service allowed");
        assert!(require_service(&ctx("user")?).is_err(), "user rejected");
        Ok(())
    }
}
