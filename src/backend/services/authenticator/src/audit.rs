//! Audit event emitter (PRD `nfr-auth-audit`, DESIGN §3.2 "Audit Emitter").
//!
//! Every auth-relevant action lands on the platform audit topic
//! (`insight.audit.events`, Redpanda) with the platform envelope (backend
//! DESIGN §3.8: JSON, versioned, `tenant_id` + `timestamp` +
//! `correlation_id` on every message; field set mirrors the Audit Service's
//! ClickHouse `insight_audit.events` schema). The Audit Service consumes and
//! stores; this side only publishes.
//!
//! Publishing is strictly non-blocking for the auth paths: `emit` drops the
//! event into a bounded channel and returns; a background task owns the
//! rdkafka producer. A full channel or a broker outage drops events (counted
//! by `auth_audit_dropped_total`) — auth availability is never coupled to
//! Redpanda availability. With no brokers configured the emitter is disabled
//! (dev stacks without Redpanda), and events still appear in the structured
//! log via the existing `target: "audit"` lines at the call sites.

use opentelemetry::metrics::Counter;
use rdkafka::ClientConfig;
use rdkafka::admin::{AdminClient, AdminOptions, NewTopic, TopicReplication};
use rdkafka::client::DefaultClientContext;
use rdkafka::producer::{FutureProducer, FutureRecord};
use serde::Serialize;
use tokio::sync::mpsc;

/// The audit envelope version tag.
const SCHEMA: &str = "insight.audit.event.v1";
/// Producer-side delivery timeout per event.
const SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// Bounded queue between auth paths and the producer task.
const QUEUE_DEPTH: usize = 1024;

/// One auth audit event, as produced by the call sites. The emitter wraps it
/// in the platform envelope (event id, timestamp, service, category, schema).
#[derive(Debug)]
pub struct AuditEvent {
    /// `login`, `session_refresh`, `logout`, `session_revoke`,
    /// `back_channel_logout`, `idp_refresh_invalid_grant`,
    /// `service_token_issued`, …
    pub action: &'static str,
    /// `success` | `failure`.
    pub outcome: &'static str,
    /// The signed tenant (empty when unresolved, e.g. a failed login).
    pub tenant_id: String,
    /// Internal person id (empty for service principals / failed logins).
    pub actor_person_id: String,
    pub actor_ip: String,
    pub actor_user_agent: String,
    /// Request correlation id (gateway `X-Correlation-Id`); empty → the
    /// envelope's `event_id` doubles as the correlation id.
    pub correlation_id: String,
    /// `session`, `service_token`, …
    pub resource_type: &'static str,
    /// Stable id of the acted-on resource (session_id, service name, …).
    pub resource_id: String,
    /// Free-form context (reason codes, counts) — serialized into `details`.
    pub details: serde_json::Value,
}

/// The wire envelope (JSON) — field names mirror the Audit Service schema.
#[derive(Debug, Serialize)]
struct Envelope {
    schema: &'static str,
    event_id: String,
    timestamp: String,
    correlation_id: String,
    tenant_id: String,
    actor_person_id: String,
    actor_ip: String,
    actor_user_agent: String,
    service: &'static str,
    action: &'static str,
    category: &'static str,
    outcome: &'static str,
    resource_type: &'static str,
    resource_id: String,
    details: String,
}

/// Best-effort: create the audit topic with `retention.ms` set, so its on-disk
/// log is bounded while no consumer drains it. If the topic already exists this
/// is a no-op (`TopicAlreadyExists` is expected and ignored) — we intentionally
/// do NOT alter an existing topic's config. Any admin/transport error is logged
/// and swallowed; auth never depends on this.
async fn ensure_topic_retention(brokers: &str, topic: &str, retention_ms: u64) {
    let admin: AdminClient<DefaultClientContext> = match ClientConfig::new()
        .set("bootstrap.servers", brokers)
        .create()
    {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(error = %e, "audit: could not build admin client for topic retention (skipping)");
            return;
        }
    };
    let retention = retention_ms.to_string();
    let new_topic =
        NewTopic::new(topic, 1, TopicReplication::Fixed(1)).set("retention.ms", &retention);
    match admin
        .create_topics([&new_topic], &AdminOptions::new())
        .await
    {
        Ok(results) => match results.into_iter().next() {
            Some(Ok(_)) => tracing::info!(
                topic,
                retention_ms,
                "audit topic created with retention (no consumer yet)"
            ),
            Some(Err((_, rdkafka::types::RDKafkaErrorCode::TopicAlreadyExists))) => {
                tracing::debug!(
                    topic,
                    "audit topic already exists; leaving its retention as-is"
                );
            }
            other => tracing::warn!(
                ?other,
                topic,
                "audit topic create returned an unexpected result (ignored)"
            ),
        },
        Err(e) => tracing::warn!(error = %e, topic, "audit topic create failed (ignored)"),
    }
}

/// Build the platform envelope for one event (pure; unit-tested).
fn envelope(event: &AuditEvent, event_id: String, timestamp: String) -> Envelope {
    let correlation_id = if event.correlation_id.is_empty() {
        event_id.clone()
    } else {
        event.correlation_id.clone()
    };
    Envelope {
        schema: SCHEMA,
        event_id,
        timestamp,
        correlation_id,
        tenant_id: event.tenant_id.clone(),
        actor_person_id: event.actor_person_id.clone(),
        actor_ip: event.actor_ip.clone(),
        actor_user_agent: event.actor_user_agent.clone(),
        service: "authenticator",
        action: event.action,
        category: "auth",
        outcome: event.outcome,
        resource_type: event.resource_type,
        resource_id: event.resource_id.clone(),
        details: event.details.to_string(),
    }
}

/// Cheap-to-clone handle; the producer lives in the background task.
#[derive(Clone)]
pub struct AuditEmitter {
    tx: Option<mpsc::Sender<AuditEvent>>,
    dropped: Counter<u64>,
}

impl AuditEmitter {
    /// Build the emitter. Empty `brokers` = disabled (a no-op handle).
    ///
    /// # Errors
    /// Fails when the Kafka producer cannot be constructed from the config
    /// (malformed broker list) — a misconfigured audit sink should fail the
    /// gear at boot, not silently drop every event.
    pub fn new(brokers: &str, topic: &str, retention_ms: u64) -> anyhow::Result<Self> {
        let meter = opentelemetry::global::meter("authenticator.audit");
        let dropped = meter
            .u64_counter("auth_audit_dropped_total")
            .with_description(
                "Audit events dropped (queue full, serialization, or delivery failure)",
            )
            .build();

        if brokers.trim().is_empty() {
            tracing::warn!(
                "audit emitter disabled: no audit.brokers configured \
                 (events remain in the structured log only)"
            );
            return Ok(Self { tx: None, dropped });
        }

        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            .set("message.timeout.ms", "5000")
            // Audit is compliance data: require the leader ack, retry inside
            // the client, keep ordering per key.
            .set("acks", "1")
            .create()
            .map_err(|e| anyhow::anyhow!("build audit producer for '{brokers}': {e}"))?;

        // Bound the topic's on-disk growth. There is NO consumer yet (the Audit
        // Service that drains → ClickHouse is spec'd but unbuilt), so events
        // would otherwise pile up at the cluster-default retention. We create
        // the topic with a short retention (default 1 day) — accepted data loss
        // until the consumer exists. Best-effort + spawned: never blocks or
        // fails auth boot, and if the topic already exists (or admin is denied)
        // it's a harmless no-op (we do NOT alter an existing topic's config,
        // to avoid clobbering infra-managed settings).
        if retention_ms > 0 {
            let brokers_owned = brokers.to_owned();
            let topic_owned = topic.to_owned();
            tokio::spawn(async move {
                ensure_topic_retention(&brokers_owned, &topic_owned, retention_ms).await;
            });
        }

        let (tx, mut rx) = mpsc::channel::<AuditEvent>(QUEUE_DEPTH);
        let topic = topic.to_owned();
        let topic_for_log = topic.clone();
        let dropped_in_task = dropped.clone();
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let env = envelope(
                    &event,
                    uuid::Uuid::now_v7().to_string(),
                    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                );
                let payload = match serde_json::to_vec(&env) {
                    Ok(p) => p,
                    Err(e) => {
                        // Count + log like every other drop path, so a
                        // malformed event still moves auth_audit_dropped_total.
                        dropped_in_task.add(1, &[]);
                        tracing::warn!(target: "audit", error = %e, action = env.action, "audit event serialization failed (dropped)");
                        continue;
                    }
                };
                // Key by tenant: per-tenant ordering, balanced partitions.
                let record = FutureRecord::to(&topic)
                    .key(&env.tenant_id)
                    .payload(&payload);
                if let Err((e, _)) = producer.send(record, SEND_TIMEOUT).await {
                    dropped_in_task.add(1, &[]);
                    tracing::warn!(target: "audit", error = %e, action = env.action, "audit event delivery failed (dropped)");
                }
            }
        });
        tracing::info!(%brokers, topic = %topic_for_log, "audit emitter started");
        Ok(Self {
            tx: Some(tx),
            dropped,
        })
    }

    /// A disabled emitter (tests / tooling).
    #[cfg(test)]
    #[must_use]
    pub fn disabled() -> Self {
        let meter = opentelemetry::global::meter("authenticator.audit");
        Self {
            tx: None,
            dropped: meter.u64_counter("auth_audit_dropped_total").build(),
        }
    }

    /// Queue one event; never blocks and never fails the caller.
    pub fn emit(&self, event: AuditEvent) {
        let Some(tx) = &self.tx else { return };
        if let Err(e) = tx.try_send(event) {
            self.dropped.add(1, &[]);
            tracing::warn!(target: "audit", error = %e, "audit queue full: event dropped");
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn event() -> AuditEvent {
        AuditEvent {
            action: "login",
            outcome: "success",
            tenant_id: "t-1".to_owned(),
            actor_person_id: "p-1".to_owned(),
            actor_ip: "10.0.0.1".to_owned(),
            actor_user_agent: "ua".to_owned(),
            correlation_id: String::new(),
            resource_type: "session",
            resource_id: "s-1".to_owned(),
            details: serde_json::json!({"idp_sub": "sub-1"}),
        }
    }

    #[test]
    fn envelope_carries_the_platform_fields() {
        let env = envelope(
            &event(),
            "evt-1".to_owned(),
            "2026-01-01T00:00:00.000Z".to_owned(),
        );
        let json = serde_json::to_value(&env).unwrap();
        for field in [
            "schema",
            "event_id",
            "timestamp",
            "correlation_id",
            "tenant_id",
            "actor_person_id",
            "actor_ip",
            "actor_user_agent",
            "service",
            "action",
            "category",
            "outcome",
            "resource_type",
            "resource_id",
            "details",
        ] {
            assert!(json.get(field).is_some(), "missing envelope field {field}");
        }
        assert_eq!(json["schema"], SCHEMA);
        assert_eq!(json["service"], "authenticator");
        assert_eq!(json["category"], "auth");
        // No explicit correlation id → the event id doubles as one.
        assert_eq!(json["correlation_id"], "evt-1");
        // details is a JSON *string* (the ClickHouse column is String).
        assert!(json["details"].is_string());
    }

    #[test]
    fn explicit_correlation_id_wins() {
        let mut e = event();
        e.correlation_id = "corr-9".to_owned();
        let env = envelope(&e, "evt-1".to_owned(), "t".to_owned());
        assert_eq!(env.correlation_id, "corr-9");
    }

    #[test]
    fn disabled_emitter_swallows_events() {
        AuditEmitter::disabled().emit(event());
    }
}
