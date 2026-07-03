//! Redis cache for person alias resolution.
//!
//! Caches Identity Resolution responses (TTL: 5 min).
//! Cache key: `person_aliases:{insight_tenant_id}:{person_id}`
//!
//! TODO: Implement Redpanda consumer for `insight.identity.resolved` topic
//! to invalidate cache on merge/split events.

// TODO: Implement Redis caching when Redis dependency is configured.
// For MVP, Identity Resolution is called on every request with person_ids filter.
// This is acceptable because:
// 1. Most queries won't filter by person_id
// 2. Person ID resolution is fast (ClickHouse lookup in Identity Resolution)
// 3. Redis adds deployment complexity for MVP

pub mod catalog_cache;

#[cfg(test)]
mod live_tests;
