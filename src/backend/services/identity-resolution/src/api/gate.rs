//! Shared admin gate for the mutating / admin identity-resolution endpoints.
//!
//! Ported from the .NET `CallerAdminCheck` + the header branch of
//! `HeaderCallerContext`: resolve the caller from the `X-Insight-Person-Id`
//! header, then require an active `admin` role in the tenant. Reused by the
//! persons-seed, roles, person-roles, and visibility endpoints.

use axum::http::HeaderMap;
use sea_orm::DatabaseConnection;
use toolkit_canonical_errors::CanonicalError;
use uuid::Uuid;

use crate::api::error::AccessError;
use crate::infra::db::roles_repo;

/// Header carrying the caller's `person_id`, parity with the .NET
/// `HeaderCallerContext`. JWT id/email-claim fallbacks are deferred until gears
/// auth carries a subject (host runs auth-disabled today — no claims).
pub(crate) const CALLER_HEADER: &str = "X-Insight-Person-Id";

/// Resolve the caller and require an active `admin` role in the tenant — the
/// .NET `CallerAdminCheck` gate. Returns the caller `person_id`, or 401 (no
/// caller) / 403 (not admin).
///
/// # Errors
///
/// 401 if no caller header, 403 if the caller is not an admin, 500 on DB error.
pub(crate) async fn require_admin(
    db: &DatabaseConnection,
    headers: &HeaderMap,
    tenant: Uuid,
) -> Result<Uuid, CanonicalError> {
    let caller = resolve_caller(headers).ok_or_else(|| {
        CanonicalError::unauthenticated()
            .with_reason(format!(
                "caller not identified; send the {CALLER_HEADER} header"
            ))
            .create()
    })?;
    let is_admin = roles_repo::has_active_admin(db, tenant, caller)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "admin role check failed");
            CanonicalError::internal("failed to verify caller permissions").create()
        })?;
    if !is_admin {
        return Err(AccessError::permission_denied()
            .with_reason("admin role required for this operation")
            .create());
    }
    Ok(caller)
}

/// Resolve the caller's `person_id` from the `X-Insight-Person-Id` header —
/// the header branch of the .NET `HeaderCallerContext` (present, parseable,
/// non-nil). Returns `None` when absent/blank/malformed/nil, which the caller
/// maps to 401. The JWT id/email-claim fallbacks are intentionally not ported
/// yet (auth-disabled host → no claims to read).
///
/// TODO(#1602): the header is caller-supplied and currently trusted (auth is
/// disabled on the host). Before prod cutover the subject MUST come from the
/// authenticated principal (gears auth / api-gateway), not a raw client header.
pub(crate) fn resolve_caller(headers: &HeaderMap) -> Option<Uuid> {
    let raw = headers.get(CALLER_HEADER)?.to_str().ok()?;
    let id = Uuid::parse_str(raw.trim()).ok()?;
    (!id.is_nil()).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(value: &str) -> anyhow::Result<HeaderMap> {
        let mut h = HeaderMap::new();
        h.insert(CALLER_HEADER, value.parse()?);
        Ok(h)
    }

    #[test]
    fn resolve_caller_reads_valid_person_header() -> anyhow::Result<()> {
        let id = Uuid::from_u128(0x1234_5678_9abc_def0);
        assert_eq!(resolve_caller(&headers_with(&id.to_string())?), Some(id));
        // Surrounding whitespace is tolerated.
        assert_eq!(
            resolve_caller(&headers_with(&format!("  {id}  "))?),
            Some(id)
        );
        Ok(())
    }

    #[test]
    fn resolve_caller_rejects_missing_blank_nil_and_malformed() -> anyhow::Result<()> {
        assert_eq!(resolve_caller(&HeaderMap::new()), None, "absent header");
        assert_eq!(resolve_caller(&headers_with("")?), None, "blank");
        assert_eq!(
            resolve_caller(&headers_with("not-a-uuid")?),
            None,
            "malformed"
        );
        assert_eq!(
            resolve_caller(&headers_with(&Uuid::nil().to_string())?),
            None,
            "nil uuid is not a caller"
        );
        Ok(())
    }
}
