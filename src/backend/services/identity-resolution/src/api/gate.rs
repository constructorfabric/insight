use sea_orm::DatabaseConnection;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::api::error::AccessError;
use crate::infra::db::roles_repo;

pub(crate) fn require_caller(ctx: &SecurityContext) -> Result<Uuid, CanonicalError> {
    let caller = ctx.subject_id();
    if caller.is_nil() {
        return Err(CanonicalError::unauthenticated()
            .with_reason("caller not identified: the gateway JWT carries no person subject")
            .create());
    }
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

pub(crate) async fn require_admin(
    db: &DatabaseConnection,
    ctx: &SecurityContext,
) -> Result<Uuid, CanonicalError> {
    let caller = require_caller(ctx)?;
    if !holds_admin(db, ctx, caller).await? {
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

pub(crate) fn require_service(ctx: &SecurityContext) -> Result<(), CanonicalError> {
    if ctx.subject_type() != Some("service") {
        return Err(AccessError::permission_denied()
            .with_reason("this endpoint is restricted to service principals (sub_type=service)")
            .create());
    }
    Ok(())
}

pub(crate) async fn holds_admin(
    db: &DatabaseConnection,
    ctx: &SecurityContext,
    caller: Uuid,
) -> Result<bool, CanonicalError> {
    roles_repo::has_active_role(
        db,
        ctx.subject_tenant_id(),
        caller,
        roles_repo::ADMIN_ROLE_ID,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "admin role check failed");
        CanonicalError::internal("failed to verify caller permissions").create()
    })
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
