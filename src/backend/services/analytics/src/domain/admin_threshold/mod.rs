//! Admin-CRUD module — owns the 5 `/v1/admin/metric-thresholds` endpoints
//! and the components behind them (Refs #525).
//!
//! Components (DESIGN §3.2):
//!
//! - [`service::AdminThresholdService`] — `cpt-metric-cat-component-admin-crud`:
//!   the validation gauntlet (authz → ref-int → shape → bounds → immutable
//!   → lock-enforcer → write → cache-invalidate → audit → schema-validator).
//! - [`lock_enforcer`] — `cpt-metric-cat-component-lock-enforcer`: broader-
//!   scope locked-row probe + the `BlockingLock` it surfaces to the service.
//! - [`audit_emitter::AuditEmitter`] — `cpt-metric-cat-component-audit-emitter`:
//!   sole writer of `threshold_lock_audit` + the derived structured-log
//!   stream. Owns the primary-vs-derived sink contract per §3.6.
//!
//! Wire shapes live in [`dto`]; SeaORM CRUD lives in [`repository`].

pub mod audit_emitter;
pub mod dto;
pub mod lock_enforcer;
pub mod repository;
pub mod service;

#[cfg(test)]
mod live_tests;

pub use service::AdminThresholdService;
