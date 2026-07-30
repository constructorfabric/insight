//! Persons-seed orchestration: tie the reader → build → group → resolve →
//! row-build → apply pipeline together. Ports the .NET `PersonsSeedService`.
//! The input source and the store are behind traits so this is unit-testable
//! with fakes (no `ClickHouse` / MariaDB).

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Serialize;
use uuid::Uuid;

use super::seed::{
    IdentityInputRow, SeedObservationRow, SourceAccountKey, assignments_to_rows, build_profiles,
    group_by_email, resolve_assignments,
};

/// Streams the raw `identity_inputs` observations for a tenant, delivered
/// **latest-first per account** (so [`build_profiles`] folds them correctly).
/// The concrete implementation reads `identity.identity_inputs` from `ClickHouse`.
#[async_trait]
pub trait IdentityInputsReader {
    async fn stream(&self, tenant_id: Uuid) -> anyhow::Result<Vec<IdentityInputRow>>;
}

/// The persons-seed store: the two resolver-feeding reads and the transactional
/// apply. Implemented over MariaDB by `infra::db::seed_repo`.
#[async_trait]
pub trait SeedStore {
    async fn known_account_bindings(
        &self,
        tenant_id: Uuid,
    ) -> anyhow::Result<HashMap<SourceAccountKey, Uuid>>;

    async fn latest_email_to_person(
        &self,
        tenant_id: Uuid,
    ) -> anyhow::Result<HashMap<String, Uuid>>;

    async fn apply(
        &self,
        tenant_id: Uuid,
        author_person_id: Uuid,
        rows: &[SeedObservationRow],
    ) -> anyhow::Result<ApplyCounts>;
}

/// Row counts from one [`SeedStore::apply`]: the net-new observations appended
/// and the `org_chart` rows rebuilt. Both are surfaced in the seed summary.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ApplyCounts {
    pub observations_inserted: u64,
    pub org_chart_rows_rebuilt: u64,
}

/// Outcome of one persons-seed run (feeds the operation status). Mirrors the
/// .NET `PersonsSeedSummary` (org-chart counter lands with that rebuild).
// Serialized field names mirror the .NET `PersonsSeedSummary` wire shape
// (`accounts_*` prefix, `accounts_minted_new`, `org_chart_rows_rebuilt`) so the
// `summary` JSON stays contract-compatible. `known_binding_conflicts` is an
// additive Insight-side field (observability of a silent identity merge; not in
// .NET) — additive keys are ignored by conformant consumers.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SeedSummary {
    pub accounts_read: usize,
    #[serde(rename = "accounts_reused_known")]
    pub reused_known: usize,
    #[serde(rename = "accounts_linked_by_email")]
    pub linked_by_email: usize,
    #[serde(rename = "accounts_minted_new")]
    pub minted: usize,
    #[serde(rename = "accounts_skipped_closed")]
    pub skipped_closed: usize,
    #[serde(rename = "accounts_skipped_no_email")]
    pub skipped_no_email: usize,
    pub observations_inserted: u64,
    pub org_chart_rows_rebuilt: u64,
    /// Email groups collapsed across a multi-person binding conflict (logged).
    pub known_binding_conflicts: usize,
}

/// Run one persons-seed: read the input stream, fold to per-account profiles,
/// group by email, resolve each group to a `person_id`, build the observation
/// rows, and apply them (append + rebuild caches). `mint` is injected so tests
/// are deterministic.
///
/// # Errors
///
/// Propagates reader / store errors.
pub async fn run_seed<R, S>(
    reader: &R,
    store: &S,
    tenant_id: Uuid,
    author_person_id: Uuid,
    mint: impl FnMut() -> Uuid,
) -> anyhow::Result<SeedSummary>
where
    R: IdentityInputsReader + ?Sized,
    S: SeedStore + ?Sized,
{
    // 1. Build per-account profiles from the (latest-first) input stream.
    let rows = reader.stream(tenant_id).await?;
    tracing::info!(input_rows = rows.len(), "persons-seed: input streamed");
    let profiles = build_profiles(rows);
    let accounts_read = profiles.len();

    // 2. Group by email; resolve each group against the current bindings/emails.
    let groups = group_by_email(profiles);
    let known = store.known_account_bindings(tenant_id).await?;
    let email_to_person = store.latest_email_to_person(tenant_id).await?;
    let outcome = resolve_assignments(groups, &known, &email_to_person, mint);
    tracing::info!(
        accounts = accounts_read,
        minted = outcome.minted,
        reused = outcome.reused_known,
        linked = outcome.linked_by_email,
        "persons-seed: resolved"
    );

    // 3. Materialize the resolved observations and apply them.
    let observation_rows = assignments_to_rows(&outcome.assignments, author_person_id);
    tracing::info!(
        observation_rows = observation_rows.len(),
        "persons-seed: applying"
    );
    let counts = store
        .apply(tenant_id, author_person_id, &observation_rows)
        .await?;
    tracing::info!(
        observations_inserted = counts.observations_inserted,
        org_chart_rows_rebuilt = counts.org_chart_rows_rebuilt,
        "persons-seed: applied"
    );

    Ok(SeedSummary {
        accounts_read,
        reused_known: outcome.reused_known,
        linked_by_email: outcome.linked_by_email,
        minted: outcome.minted,
        skipped_closed: outcome.skipped_closed,
        skipped_no_email: outcome.skipped_no_email,
        observations_inserted: counts.observations_inserted,
        org_chart_rows_rebuilt: counts.org_chart_rows_rebuilt,
        known_binding_conflicts: outcome.known_binding_conflicts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::prelude::DateTime;

    struct FakeReader {
        rows: Vec<IdentityInputRow>,
    }
    #[async_trait]
    impl IdentityInputsReader for FakeReader {
        async fn stream(&self, _tenant: Uuid) -> anyhow::Result<Vec<IdentityInputRow>> {
            Ok(self.rows.clone())
        }
    }

    struct FakeStore {
        known: HashMap<SourceAccountKey, Uuid>,
        emails: HashMap<String, Uuid>,
    }
    #[async_trait]
    impl SeedStore for FakeStore {
        async fn known_account_bindings(
            &self,
            _tenant: Uuid,
        ) -> anyhow::Result<HashMap<SourceAccountKey, Uuid>> {
            Ok(self.known.clone())
        }
        async fn latest_email_to_person(
            &self,
            _tenant: Uuid,
        ) -> anyhow::Result<HashMap<String, Uuid>> {
            Ok(self.emails.clone())
        }
        async fn apply(
            &self,
            _tenant: Uuid,
            _author: Uuid,
            rows: &[SeedObservationRow],
        ) -> anyhow::Result<ApplyCounts> {
            // Net-inserted (no dedup in the fake); org_chart rebuild is DB-only.
            Ok(ApplyCounts {
                observations_inserted: rows.len() as u64,
                org_chart_rows_rebuilt: 0,
            })
        }
    }

    fn input(src: &str, acct: &str, vt: &str, val: &str, t: DateTime) -> IdentityInputRow {
        IdentityInputRow {
            source_type: src.to_owned(),
            source_id: Uuid::from_u128(1),
            source_account_id: acct.to_owned(),
            value_type: vt.to_owned(),
            value: val.to_owned(),
            synced_at: t,
            is_delete: false,
        }
    }

    fn counter() -> impl FnMut() -> Uuid {
        let mut n = 0u128;
        move || {
            n += 1;
            Uuid::from_u128(n)
        }
    }

    #[tokio::test]
    async fn run_seed_wires_pipeline_end_to_end() -> anyhow::Result<()> {
        let t: DateTime = "2026-01-01T00:00:00".parse()?;
        // Anna across two sources (shared email) + Boris; empty store → all mint.
        let reader = FakeReader {
            rows: vec![
                input("bamboohr", "5001", "email", "anna@corp.com", t),
                input("bamboohr", "5001", "display_name", "Anna P", t),
                input("slack", "U777", "email", "anna@corp.com", t),
                input("bamboohr", "5000", "email", "boris@corp.com", t),
            ],
        };
        let store = FakeStore {
            known: HashMap::new(),
            emails: HashMap::new(),
        };

        let summary = run_seed(
            &reader,
            &store,
            Uuid::from_u128(9),
            Uuid::from_u128(99),
            counter(),
        )
        .await?;

        assert_eq!(summary.accounts_read, 3, "5001 + U777 + 5000");
        assert_eq!(
            summary.minted, 3,
            "counts profiles across the minted groups"
        );
        assert_eq!(summary.reused_known, 0);
        assert_eq!(summary.linked_by_email, 0);
        // Anna: email+display_name (5001) + email (U777) = 3; Boris: email = 1.
        assert_eq!(summary.observations_inserted, 4);
        Ok(())
    }

    #[tokio::test]
    async fn run_seed_reuses_known_binding() -> anyhow::Result<()> {
        let t: DateTime = "2026-01-01T00:00:00".parse()?;
        let reader = FakeReader {
            rows: vec![input("bamboohr", "5000", "email", "boris@corp.com", t)],
        };
        let mut known = HashMap::new();
        known.insert(
            SourceAccountKey {
                source_type: "bamboohr".to_owned(),
                source_id: Uuid::from_u128(1),
                account_id: "5000".to_owned(),
            },
            Uuid::from_u128(7),
        );
        let store = FakeStore {
            known,
            emails: HashMap::new(),
        };

        let summary = run_seed(
            &reader,
            &store,
            Uuid::from_u128(9),
            Uuid::from_u128(99),
            counter(),
        )
        .await?;
        assert_eq!(summary.reused_known, 1);
        assert_eq!(summary.minted, 0);
        assert_eq!(summary.observations_inserted, 1);
        Ok(())
    }
}
