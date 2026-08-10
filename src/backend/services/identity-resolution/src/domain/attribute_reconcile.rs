use async_trait::async_trait;
use serde::Serialize;
use uuid::Uuid;

use crate::infra::db::person_attributes_repo::{DefinitionKey, RegisterOutcome};

#[derive(Debug, Clone)]
pub struct DiscoveredField {
    pub insight_tenant_id: String,
    pub insight_source_type: String,
    pub insight_source_id: String,
    pub source_field_id: String,
    pub last_observed_at: String,
}

#[async_trait]
pub trait DiscoveredFieldsReader {
    async fn discover(&self) -> anyhow::Result<Vec<DiscoveredField>>;
}

#[async_trait]
pub trait FieldRegistrar {
    async fn register(
        &self,
        key: &DefinitionKey,
        observed_at: &str,
    ) -> anyhow::Result<RegisterOutcome>;
}

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct ReconcileSummary {
    pub discovered: usize,
    pub created: usize,
    pub refreshed: usize,
    pub skipped_invalid: usize,
    pub non_canonical_tenants: usize,
}

#[derive(Debug)]
pub enum ReconcileError {
    Guard(String),
    Failed(anyhow::Error),
}

impl From<anyhow::Error> for ReconcileError {
    fn from(e: anyhow::Error) -> Self {
        Self::Failed(e)
    }
}

fn is_canonical_uuid(tenant: &str) -> bool {
    Uuid::parse_str(tenant).is_ok_and(|u| u.to_string() == tenant)
}

fn to_key(field: &DiscoveredField) -> Option<DefinitionKey> {
    let all_present = !field.insight_tenant_id.is_empty()
        && !field.insight_source_type.is_empty()
        && !field.insight_source_id.is_empty()
        && !field.source_field_id.is_empty();
    all_present.then(|| DefinitionKey {
        insight_tenant_id: field.insight_tenant_id.clone(),
        insight_source_type: field.insight_source_type.clone(),
        insight_source_id: field.insight_source_id.clone(),
        source_field_id: field.source_field_id.clone(),
    })
}

pub async fn run_reconcile(
    reader: &dyn DiscoveredFieldsReader,
    registrar: &dyn FieldRegistrar,
) -> Result<ReconcileSummary, ReconcileError> {
    let fields = reader.discover().await?;

    let mut summary = ReconcileSummary {
        discovered: fields.len(),
        ..ReconcileSummary::default()
    };
    for field in &fields {
        let Some(key) = to_key(field) else {
            summary.skipped_invalid += 1;
            continue;
        };
        if !is_canonical_uuid(&key.insight_tenant_id) {
            summary.non_canonical_tenants += 1;
        }
        match registrar.register(&key, &field.last_observed_at).await? {
            RegisterOutcome::Created => summary.created += 1,
            RegisterOutcome::Refreshed => summary.refreshed += 1,
        }
    }

    if summary.discovered > 0 && summary.created + summary.refreshed == 0 {
        return Err(ReconcileError::Guard(format!(
            "all {} discovered fields had empty key components; refusing to \
             complete a run that registered nothing — the claims contract is \
             likely broken",
            summary.discovered
        )));
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct FakeReader(Vec<DiscoveredField>);

    #[async_trait]
    impl DiscoveredFieldsReader for FakeReader {
        async fn discover(&self) -> anyhow::Result<Vec<DiscoveredField>> {
            Ok(self.0.clone())
        }
    }

    struct FakeRegistrar {
        seen: Mutex<Vec<DefinitionKey>>,
        outcome: RegisterOutcome,
    }

    #[async_trait]
    impl FieldRegistrar for FakeRegistrar {
        async fn register(
            &self,
            key: &DefinitionKey,
            _observed_at: &str,
        ) -> anyhow::Result<RegisterOutcome> {
            self.seen
                .lock()
                .map_err(|_| anyhow::anyhow!("poisoned"))?
                .push(key.clone());
            Ok(self.outcome)
        }
    }

    fn field(tenant: &str, field_id: &str) -> DiscoveredField {
        DiscoveredField {
            insight_tenant_id: tenant.to_owned(),
            insight_source_type: "bamboohr".to_owned(),
            insight_source_id: "hr-main".to_owned(),
            source_field_id: field_id.to_owned(),
            last_observed_at: "2026-01-01 00:00:00.000".to_owned(),
        }
    }

    #[tokio::test]
    async fn registers_every_valid_field_and_counts_outcomes() {
        let reader = FakeReader(vec![field("t", "jobTitle"), field("t", "department")]);
        let registrar = FakeRegistrar {
            seen: Mutex::new(vec![]),
            outcome: RegisterOutcome::Created,
        };

        let summary = run_reconcile(&reader, &registrar).await.ok();

        assert_eq!(
            summary,
            Some(ReconcileSummary {
                discovered: 2,
                created: 2,
                refreshed: 0,
                skipped_invalid: 0,
                non_canonical_tenants: 2,
            })
        );
    }

    #[tokio::test]
    async fn empty_key_components_are_skipped_not_registered() {
        let reader = FakeReader(vec![field("", "jobTitle"), field("t", "department")]);
        let registrar = FakeRegistrar {
            seen: Mutex::new(vec![]),
            outcome: RegisterOutcome::Refreshed,
        };

        let summary = run_reconcile(&reader, &registrar).await.ok();

        assert_eq!(
            summary,
            Some(ReconcileSummary {
                discovered: 2,
                created: 0,
                refreshed: 1,
                skipped_invalid: 1,
                non_canonical_tenants: 1,
            })
        );
    }

    #[tokio::test]
    async fn all_invalid_fields_refuse_instead_of_completing_green() {
        let reader = FakeReader(vec![field("", "jobTitle")]);
        let registrar = FakeRegistrar {
            seen: Mutex::new(vec![]),
            outcome: RegisterOutcome::Created,
        };

        let refusal = run_reconcile(&reader, &registrar).await;

        assert!(matches!(refusal, Err(ReconcileError::Guard(_))));
    }

    #[tokio::test]
    async fn canonical_uuid_tenants_are_not_flagged() {
        let mut f = field("0193e6ae-7c2d-7b1a-9e4f-0a1b2c3d4e5f", "jobTitle");
        f.insight_tenant_id = f.insight_tenant_id.to_lowercase();
        let reader = FakeReader(vec![f]);
        let registrar = FakeRegistrar {
            seen: Mutex::new(vec![]),
            outcome: RegisterOutcome::Created,
        };

        let summary = run_reconcile(&reader, &registrar).await.ok();

        assert_eq!(
            summary.map(|s| s.non_canonical_tenants),
            Some(0),
            "canonical lowercase UUID must not be flagged"
        );
    }

    #[tokio::test]
    async fn empty_discovery_completes_as_noop() {
        let reader = FakeReader(vec![]);
        let registrar = FakeRegistrar {
            seen: Mutex::new(vec![]),
            outcome: RegisterOutcome::Created,
        };

        let summary = run_reconcile(&reader, &registrar).await.ok();

        assert_eq!(summary, Some(ReconcileSummary::default()));
    }
}
