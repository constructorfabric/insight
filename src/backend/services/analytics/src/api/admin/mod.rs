//! `/v1/admin/metric-thresholds/*` HTTP layer (Refs #525).
//!
//! Thin: handlers do JSON-extractor wiring + delegate to
//! [`crate::domain::admin_threshold::AdminThresholdService`], which owns
//! the entire validation gauntlet and returns either the wire payload or
//! a fully-formed canonical `Response`. No envelope construction lives in
//! handlers.
//!
//! ## Why handlers can't be even thinner
//!
//! The list handler uses `axum::extract::Query<ListFilters>`, which
//! produces a non-canonical `QueryRejection` on parse failure. We need
//! to translate that into the canonical envelope so the
//! "`?tenant_id=...` → 400 `invalid_argument`" contract holds at the wire
//! — same shape body-parse failures already use through `CanonicalJson`.
//! That's the single piece of envelope translation we do in this module;
//! everything else flows through the service.

pub(crate) mod error_map;
mod handlers;

pub(crate) use handlers::{create, delete, get_one, list, update};
