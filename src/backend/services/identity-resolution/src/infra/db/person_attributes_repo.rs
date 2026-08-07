//! Person-attribute registry: connector-discovered attribute definitions and
//! their append-only policy revisions.
//!
//! Definitions are keyed by the RAW string identifiers the warehouse claim
//! relations carry (see the deviation note in `sql/015_person_attributes.sql`).
//! Policy revisions never mutate; the current policy is the row with the
//! highest revision per definition, and every revision carries its actor.

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, SqlErr, Statement};
use uuid::Uuid;

/// Stable identity of one discovered source field.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DefinitionKey {
    pub insight_tenant_id: String,
    pub insight_source_type: String,
    pub insight_source_id: String,
    pub source_field_id: String,
}

/// One registry row joined with its current (highest-revision) policy.
#[derive(Debug, Clone)]
pub struct DefinitionWithPolicy {
    pub id: Uuid,
    pub key: DefinitionKey,
    pub first_observed_at: String,
    pub last_observed_at: String,
    pub policy: PolicyRevision,
}

/// One immutable policy revision (read model).
#[derive(Debug, Clone)]
pub struct PolicyRevision {
    pub revision: i32,
    pub label_override: Option<String>,
    pub sensitivity_class: Option<String>,
    pub grouping_enabled: bool,
    pub comparison_enabled: bool,
    pub value_mode: ValueMode,
    pub retired: bool,
    pub actor_person_id: Uuid,
    pub reason: String,
}

/// Policy fields a caller may set; revision and actor are assigned by the
/// append itself.
#[derive(Debug, Clone)]
pub struct PolicyInput {
    pub label_override: Option<String>,
    pub sensitivity_class: Option<String>,
    pub grouping_enabled: bool,
    pub comparison_enabled: bool,
    pub value_mode: ValueMode,
    pub retired: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueMode {
    Single,
    Multi,
}

impl ValueMode {
    #[must_use]
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Multi => "multi",
        }
    }

    /// # Errors
    ///
    /// Returns an error for a value outside the `value_mode` enum.
    pub fn from_db(s: &str) -> anyhow::Result<Self> {
        match s {
            "single" => Ok(Self::Single),
            "multi" => Ok(Self::Multi),
            other => anyhow::bail!("unknown value_mode: {other}"),
        }
    }
}

/// Outcome of registering one discovered field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterOutcome {
    Created,
    Refreshed,
}

/// Register a discovered field: insert the definition plus its revision-1
/// default policy (grouping allowed, comparison denied) when the key is new;
/// otherwise only advance `last_observed_at`. Idempotent per key.
///
/// The insert and its revision-1 row are two statements without a wrapping
/// transaction: a crash between them leaves a definition whose revision 1 is
/// re-inserted by the next run (the `INSERT IGNORE` below), never a definition
/// that reconciliation refuses to touch again.
///
/// The outcome is classified from the `INSERT IGNORE` (1 row = the field had
/// no revision 1, so it registers as created): the definition insert's
/// affected-rows count is unreliable under `CLIENT_FOUND_ROWS`, which the
/// driver enables — a duplicate-key no-op reports 1 there, not 0.
///
/// # Errors
///
/// Returns an error if a statement fails.
pub async fn register_discovered(
    db: &DatabaseConnection,
    key: &DefinitionKey,
    observed_at: &str,
    system_actor: Uuid,
) -> anyhow::Result<RegisterOutcome> {
    const INSERT_DEFINITION: &str = r"
        INSERT INTO person_attribute_definitions
            (id, insight_tenant_id, insight_source_type, insight_source_id,
             source_field_id, first_observed_at, last_observed_at)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        ON DUPLICATE KEY UPDATE
            last_observed_at = GREATEST(last_observed_at, VALUES(last_observed_at))
    ";
    const INSERT_INITIAL_POLICY: &str = r"
        INSERT IGNORE INTO person_attribute_policy_revisions
            (id, definition_id, revision, actor_person_id, reason)
        SELECT ?, d.id, 1, ?, 'registered by attribute reconciliation'
        FROM person_attribute_definitions d
        WHERE d.insight_tenant_id   = ?
          AND d.insight_source_type = ?
          AND d.insight_source_id   = ?
          AND d.source_field_id     = ?
    ";

    let definition_id = Uuid::now_v7();
    let insert = Statement::from_sql_and_values(
        DbBackend::MySql,
        INSERT_DEFINITION,
        [
            definition_id.as_bytes().to_vec().into(),
            key.insight_tenant_id.clone().into(),
            key.insight_source_type.clone().into(),
            key.insight_source_id.clone().into(),
            key.source_field_id.clone().into(),
            observed_at.into(),
            observed_at.into(),
        ],
    );
    db.execute(insert).await?;

    let policy = Statement::from_sql_and_values(
        DbBackend::MySql,
        INSERT_INITIAL_POLICY,
        [
            Uuid::now_v7().as_bytes().to_vec().into(),
            system_actor.as_bytes().to_vec().into(),
            key.insight_tenant_id.clone().into(),
            key.insight_source_type.clone().into(),
            key.insight_source_id.clone().into(),
            key.source_field_id.clone().into(),
        ],
    );
    let policy_result = db.execute(policy).await?;

    Ok(if policy_result.rows_affected() == 1 {
        RegisterOutcome::Created
    } else {
        RegisterOutcome::Refreshed
    })
}

/// Append the next policy revision iff the caller saw the current one.
/// Returns `false` when `expected_revision` is stale (or the definition does
/// not exist / belongs to another tenant) — the API maps that to 409. The
/// guarded `INSERT ... SELECT` makes check-and-insert one statement; if two
/// writers still race past it, `uq_definition_revision` rejects the loser,
/// which is reported as the same stale-revision outcome rather than an error.
///
/// # Errors
///
/// Returns an error if the statement fails for any reason other than the
/// revision uniqueness key.
pub async fn append_policy_revision(
    db: &DatabaseConnection,
    tenant_id: &str,
    definition_id: Uuid,
    expected_revision: i32,
    policy: &PolicyInput,
    actor_person_id: Uuid,
) -> anyhow::Result<bool> {
    const SQL: &str = r"
        INSERT INTO person_attribute_policy_revisions
            (id, definition_id, revision, label_override, sensitivity_class,
             grouping_enabled, comparison_enabled, value_mode, retired,
             actor_person_id, reason)
        SELECT ?, d.id, ?, ?, ?, ?, ?, ?, ?, ?, ?
        FROM person_attribute_definitions d
        WHERE d.id = ?
          AND d.insight_tenant_id = ?
          AND ? = (SELECT MAX(r.revision)
                   FROM person_attribute_policy_revisions r
                   WHERE r.definition_id = d.id)
    ";

    let next = expected_revision
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("revision overflow"))?;
    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            Uuid::now_v7().as_bytes().to_vec().into(),
            next.into(),
            policy.label_override.clone().into(),
            policy.sensitivity_class.clone().into(),
            policy.grouping_enabled.into(),
            policy.comparison_enabled.into(),
            policy.value_mode.as_db().into(),
            policy.retired.into(),
            actor_person_id.as_bytes().to_vec().into(),
            policy.reason.clone().into(),
            definition_id.as_bytes().to_vec().into(),
            tenant_id.into(),
            expected_revision.into(),
        ],
    );

    match db.execute(stmt).await {
        Ok(result) => Ok(result.rows_affected() == 1),
        Err(err) if is_duplicate_key(&err) => Ok(false),
        Err(err) => Err(err.into()),
    }
}

fn is_duplicate_key(err: &sea_orm::DbErr) -> bool {
    matches!(err.sql_err(), Some(SqlErr::UniqueConstraintViolation(_)))
}

/// List a tenant's definitions with their current policy, ordered by source
/// then field for stable presentation.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn list_definitions(
    db: &DatabaseConnection,
    tenant_id: &str,
) -> anyhow::Result<Vec<DefinitionWithPolicy>> {
    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        format!(
            "{SELECT_WITH_POLICY} WHERE d.insight_tenant_id = ? ORDER BY d.insight_source_type, d.insight_source_id, d.source_field_id"
        ),
        [tenant_id.into()],
    );
    let rows = db.query_all(stmt).await?;
    rows.iter().map(row_to_definition).collect()
}

/// One definition with its current policy, tenant-scoped.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn get_definition(
    db: &DatabaseConnection,
    tenant_id: &str,
    definition_id: Uuid,
) -> anyhow::Result<Option<DefinitionWithPolicy>> {
    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        format!("{SELECT_WITH_POLICY} WHERE d.insight_tenant_id = ? AND d.id = ?"),
        [tenant_id.into(), definition_id.as_bytes().to_vec().into()],
    );
    let row = db.query_one(stmt).await?;
    row.as_ref().map(row_to_definition).transpose()
}

const SELECT_WITH_POLICY: &str = r"
    SELECT
        d.id                    AS definition_id,
        d.insight_tenant_id     AS tenant_id,
        d.insight_source_type   AS source_type,
        d.insight_source_id     AS source_instance,
        d.source_field_id       AS field_id,
        CAST(d.first_observed_at AS CHAR) AS first_observed,
        CAST(d.last_observed_at  AS CHAR) AS last_observed,
        r.revision              AS revision,
        r.label_override        AS label_override,
        r.sensitivity_class     AS sensitivity_class,
        r.grouping_enabled      AS grouping_enabled,
        r.comparison_enabled    AS comparison_enabled,
        r.value_mode            AS value_mode,
        r.retired               AS retired,
        r.actor_person_id       AS actor_person_id,
        r.reason                AS reason
    FROM person_attribute_definitions d
    JOIN person_attribute_policy_revisions r
        ON r.definition_id = d.id
        AND r.revision = (SELECT MAX(r2.revision)
                          FROM person_attribute_policy_revisions r2
                          WHERE r2.definition_id = d.id)
";

fn row_to_definition(row: &sea_orm::QueryResult) -> anyhow::Result<DefinitionWithPolicy> {
    let id: Vec<u8> = row.try_get("", "definition_id")?;
    let actor: Vec<u8> = row.try_get("", "actor_person_id")?;
    let value_mode: String = row.try_get("", "value_mode")?;
    Ok(DefinitionWithPolicy {
        id: Uuid::from_slice(&id)?,
        key: DefinitionKey {
            insight_tenant_id: row.try_get("", "tenant_id")?,
            insight_source_type: row.try_get("", "source_type")?,
            insight_source_id: row.try_get("", "source_instance")?,
            source_field_id: row.try_get("", "field_id")?,
        },
        first_observed_at: row.try_get("", "first_observed")?,
        last_observed_at: row.try_get("", "last_observed")?,
        policy: PolicyRevision {
            revision: row.try_get("", "revision")?,
            label_override: row.try_get("", "label_override")?,
            sensitivity_class: row.try_get("", "sensitivity_class")?,
            grouping_enabled: row.try_get("", "grouping_enabled")?,
            comparison_enabled: row.try_get("", "comparison_enabled")?,
            value_mode: ValueMode::from_db(&value_mode)?,
            retired: row.try_get("", "retired")?,
            actor_person_id: Uuid::from_slice(&actor)?,
            reason: row.try_get("", "reason")?,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::ValueMode;

    #[test]
    fn value_mode_round_trips_through_db_representation() {
        for mode in [ValueMode::Single, ValueMode::Multi] {
            assert_eq!(
                ValueMode::from_db(mode.as_db()).ok(),
                Some(mode),
                "should round-trip: {mode:?}"
            );
        }
    }

    #[test]
    fn value_mode_rejects_unknown_values() {
        assert!(ValueMode::from_db("plural").is_err());
    }
}
