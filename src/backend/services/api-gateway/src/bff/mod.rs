//! Backend-for-Frontend (BFF) module.
//!
//! Owns the OIDC handshake, server-side session lifecycle, and `/auth/*`
//! API consumed by the SPA. See
//! `docs/components/backend/api-gateway/bff/{PRD,DESIGN}.md`.

pub mod audit;
pub mod config;
pub mod cookies;
pub mod errors;
pub mod handlers;
pub mod identity;
pub mod module;
pub mod oidc_client;
pub mod redis_keys;
pub mod routes;
pub mod secrets;
pub mod session;
pub mod session_store;

// `BffModule` is registered into the inventory by `#[modkit::module]` —
// no explicit re-export needed here.
