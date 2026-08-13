//! Read queries against the identity store (`persons`).
//!
//! Ported from the .NET service's `Sql.Profiles.cs`. The resolution queries use
//! window functions (`ROW_NUMBER()` over the canonical partition) that have no
//! first-class SeaORM query-builder form and no `toolkit-db` equivalent (see
//! `infra::db` module docs + constructorfabric/gears-rust#4239), so we run them
//! as **raw SQL** via SeaORM's `Statement` and read columns off the
//! `QueryResult`. Running the same SQL as the .NET service keeps resolution
//! behaviour identical — with ONE deliberate deviation the .NET service never
//! needed: rows naming the excluded-person sentinel (ADR-0003; only the Rust
//! correction verbs can mint them) are filtered AFTER the latest-wins ranking,
//! so an excluded account resolves as no person rather than as the shared
//! sentinel, and an older binding is never resurrected past an exclusion.

use std::collections::HashMap;

use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, QueryFilter,
    QueryResult, QuerySelect, Statement,
};
use uuid::Uuid;

use super::entities::persons;
use crate::domain::person_card::{self, CARD_VALUE_TYPES, PersonCard};
use crate::domain::resolution::EXCLUDED_PERSON;

/// Resolve the set of `person_id`s whose CURRENT email (latest observation per
/// source instance) equals `email` within the tenant.
///
/// The caller maps the result to the contract: 0 rows → 404 `person_not_found`,
/// 1 → resolved, >1 → 422 `ambiguous_profile`.
///
/// Case handling matches the .NET service (ADR-0011): the input is trimmed
/// only — the `value_id` column collation does case-insensitive matching.
///
/// # Errors
///
/// Returns an error if the query fails or a stored `person_id` is not 16 bytes.
pub async fn resolve_person_ids_by_email(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    email: &str,
) -> anyhow::Result<Vec<Uuid>> {
    // Verbatim from Sql.Profiles.cs::ResolvePersonIdsByEmail, `@param` -> `?`.
    const SQL: &str = r"
        WITH ranked AS (
            SELECT
                person_id,
                value_id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, person_id, insight_source_type, insight_source_id, value_type
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE insight_tenant_id = ?
              AND value_type = 'email'
        )
        SELECT DISTINCT person_id
        FROM ranked
        WHERE rn = 1
          AND value_id = ?
          AND person_id != ?
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            tenant_id.as_bytes().to_vec().into(),
            email.trim().to_owned().into(),
            EXCLUDED_PERSON.as_bytes().to_vec().into(),
        ],
    );

    let rows = db.query_all(stmt).await?;
    person_ids_from_rows(rows)
}

/// Tenant-AGNOSTIC email → `person_id` resolution for the authenticator's
/// admin `__override` (view-as) service-only lookup
/// (`GET /internal/persons/by-email-override`) ONLY — the login bootstrap
/// resolves by external id (see `resolve_person_id_by_source_any_tenant`
/// below), NEVER by email. At override time neither the target's tenant is
/// yet known, so the tenant filter is dropped and any matching tenant's
/// latest observation wins. Returns the single winning `person_id`, or
/// `None` when the email is unknown. Ported
/// verbatim from `Sql.cs::ResolvePersonIdByEmailAnyTenant` (window `ROW_NUMBER()`
/// → raw SQL, see `infra::db` module docs + constructorfabric/gears-rust#4239).
///
/// # Errors
///
/// Returns an error if the query fails or a stored `person_id` is not 16 bytes.
pub async fn resolve_person_id_by_email_any_tenant(
    db: &DatabaseConnection,
    email: &str,
) -> anyhow::Result<Option<Uuid>> {
    const SQL: &str = r"
        WITH ranked AS (
            SELECT
                person_id,
                id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, insight_source_type, insight_source_id, value_type, value_id
                    ORDER BY created_at DESC, id DESC
                ) AS rn,
                created_at
            FROM persons
            WHERE value_type = 'email'
              AND value_id = ?
        )
        SELECT person_id
        FROM ranked
        WHERE rn = 1
          AND person_id != ?
        ORDER BY created_at DESC, id DESC
        LIMIT 1
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            email.trim().to_owned().into(),
            EXCLUDED_PERSON.as_bytes().to_vec().into(),
        ],
    );

    match db.query_one(stmt).await? {
        Some(row) => {
            let bytes: Vec<u8> = row.try_get("", "person_id")?;
            Ok(Some(Uuid::from_slice(&bytes)?))
        }
        None => Ok(None),
    }
}

/// Tenant-AGNOSTIC `(source_type, external_id)` → `person_id` resolution for the
/// login bootstrap ONLY (the authenticator's service-only
/// `GET /internal/persons/by-external-id?source_type=...&external_id=...`
/// call — a SEPARATE route from the email-override lookup above): at login
/// the caller's tenant is not yet known, so unlike
/// `resolve_person_ids_by_source_id` this does not scope by a per-tenant
/// `insight_source_id` (connector instance) — only the source **type** (e.g.
/// `ms-entra`) and the source-native external id are known ahead of tenant
/// resolution. Returns the single winning `person_id`, or `None` when the pair
/// is unknown. Same latest-observation-wins semantics as
/// `resolve_person_id_by_email_any_tenant`.
///
/// SECURITY INVARIANT this relies on (same one email-based any-tenant lookup
/// already relied on): `(source_type, external_id)` must be unique across
/// every tenant sharing this database, or the wrong tenant's person could
/// win. This holds today because `idp.source_type`/`issuer_url` are ONE
/// value per authenticator deployment — every login for every tenant behind
/// it goes through the SAME real external IdP, so `external_id` is only as
/// unique as that one IdP's own id space (Entra `oid` / Keycloak's internal
/// user id are effectively globally unique within their own directory).
/// Revisit this comment when multi-IdP config lands (constructorfabric/insight#1960
/// follow-ups) — that's the point where two DIFFERENT real `IdPs` could
/// plausibly share a `source_type` label and this invariant needs an
/// explicit uniqueness check, not just "it happens to hold today".
///
/// # Errors
///
/// Returns an error if the query fails or a stored `person_id` is not 16 bytes.
pub async fn resolve_person_id_by_source_any_tenant(
    db: &DatabaseConnection,
    source_type: &str,
    external_id: &str,
) -> anyhow::Result<Option<Uuid>> {
    const SQL: &str = r"
        WITH ranked AS (
            SELECT
                person_id,
                id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, insight_source_type, insight_source_id, value_type, value_id
                    ORDER BY created_at DESC, id DESC
                ) AS rn,
                created_at
            FROM persons
            WHERE value_type = 'id'
              AND insight_source_type = ?
              AND value_id = ?
        )
        SELECT person_id
        FROM ranked
        WHERE rn = 1
          AND person_id != ?
        ORDER BY created_at DESC, id DESC
        LIMIT 1
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            source_type.to_owned().into(),
            external_id.to_owned().into(),
            EXCLUDED_PERSON.as_bytes().to_vec().into(),
        ],
    );

    match db.query_one(stmt).await? {
        Some(row) => {
            let bytes: Vec<u8> = row.try_get("", "person_id")?;
            Ok(Some(Uuid::from_slice(&bytes)?))
        }
        None => Ok(None),
    }
}

/// Resolve the set of `person_id`s whose CURRENT `value_type='id'` observation
/// on the given source instance (`source_type` + `source_id`) equals `value`.
/// Source-instance scoped, ported from .NET `Sql.Profiles.cs::ResolvePersonIdsBySourceId`.
///
/// # Errors
///
/// Returns an error if the query fails or a stored `person_id` is not 16 bytes.
pub async fn resolve_person_ids_by_source_id(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    source_type: &str,
    source_id: Uuid,
    value: &str,
) -> anyhow::Result<Vec<Uuid>> {
    const SQL: &str = r"
        WITH ranked AS (
            SELECT
                person_id,
                value_id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, person_id, insight_source_type, insight_source_id, value_type
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE insight_tenant_id   = ?
              AND insight_source_type = ?
              AND insight_source_id   = ?
              AND value_type          = 'id'
        )
        SELECT DISTINCT person_id
        FROM ranked
        WHERE rn = 1
          AND value_id = ?
          AND person_id != ?
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            tenant_id.as_bytes().to_vec().into(),
            source_type.to_owned().into(),
            source_id.as_bytes().to_vec().into(),
            // Source-native ids are matched as-is (the .NET service trims only
            // email, not the id path).
            value.to_owned().into(),
            EXCLUDED_PERSON.as_bytes().to_vec().into(),
        ],
    );

    let rows = db.query_all(stmt).await?;
    person_ids_from_rows(rows)
}

/// Whether the tenant's persons log holds any observation for `person_id`.
///
/// The existence question the `value_type='person_id'` profile lookup needs:
/// the log is the person registry, so "has at least one row" IS "the person
/// exists in this tenant". Kept as a bounded EXISTS-shaped probe rather than
/// reusing `fetch_person_observations`, which pulls every row of the person.
///
/// # Errors
///
/// Returns an error if the query fails.
/// The subset of `person_ids` with at least one observation in the tenant's
/// persons log — the batch form of [`person_exists`], for the wildcard branch
/// of the visible-persons filter: a wildcard grant covers everyone IN THE
/// TENANT, so ids from another tenant (or from nowhere) must not be echoed
/// back as visible.
///
/// # Errors
///
/// Returns an error if the query fails or a stored `person_id` is not 16 bytes.
pub async fn persons_in_tenant(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_ids: &[Uuid],
) -> anyhow::Result<Vec<Uuid>> {
    if person_ids.is_empty() {
        return Ok(Vec::new());
    }

    let rows = persons::Entity::find()
        .select_only()
        .column(persons::Column::PersonId)
        .filter(persons::Column::InsightTenantId.eq(tenant_id.as_bytes().to_vec()))
        .filter(persons::Column::PersonId.is_in(person_ids.iter().map(|id| id.as_bytes().to_vec())))
        .filter(persons::Column::PersonId.ne(EXCLUDED_PERSON.as_bytes().to_vec()))
        .distinct()
        .into_tuple::<Vec<u8>>()
        .all(db)
        .await?;

    rows.iter().map(|raw| Ok(Uuid::from_slice(raw)?)).collect()
}

pub async fn person_exists(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_id: Uuid,
) -> anyhow::Result<bool> {
    // The excluded-person sentinel accumulates journal rows once any account
    // is excluded, but it is not a person and the read API must not serve it.
    if person_id == EXCLUDED_PERSON {
        return Ok(false);
    }

    let found = persons::Entity::find()
        .filter(persons::Column::InsightTenantId.eq(tenant_id.as_bytes().to_vec()))
        .filter(persons::Column::PersonId.eq(person_id.as_bytes().to_vec()))
        .one(db)
        .await?;
    Ok(found.is_some())
}

/// Find persons whose CURRENT observed values match every search term
/// (case-insensitive substring). "Current" is the same window rule
/// [`resolve_person_ids_by_email`] applies — the latest observation per
/// person × source × value type — so a value superseded by that person's own
/// newer data stops matching, while a value two persons both currently claim
/// returns both. Ordered by `person_id` for a stable page; the API layer
/// re-sorts for display.
///
/// INVARIANT: this is a deliberate tenant-bounded scan, not a missing index.
/// A substring match over the journal cannot use a B-tree, and a derived
/// current-values table would be one more cache to keep in sync with every
/// write path. The consumer is the admin picker — one debounced human — and
/// the scan is bounded by the tenant filter and the six searchable value
/// types. Revisit as a derived table only if measured slow at real scale.
///
/// # Errors
///
/// Returns an error if the query fails or a stored `person_id` is not 16 bytes.
pub async fn search_persons_by_current_values(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    terms: &[String],
    limit: u64,
) -> anyhow::Result<Vec<Uuid>> {
    if terms.is_empty() {
        return Ok(Vec::new());
    }

    // One EXISTS per term: every term must match SOME current value of the
    // person, not necessarily the same one ("iva example.com" finds a person
    // via name + email together).
    let mut sql = String::from(
        r"
        WITH ranked AS (
            SELECT
                person_id,
                value_effective,
                ROW_NUMBER() OVER (
                    PARTITION BY person_id, insight_source_type, insight_source_id, value_type
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE insight_tenant_id = ?
              AND value_type IN ('email', 'username', 'display_name',
                                 'first_name', 'last_name', 'employee_id')
        ),
        current_vals AS (
            SELECT person_id, value_effective
            FROM ranked
            WHERE rn = 1 AND value_effective IS NOT NULL
        )
        SELECT DISTINCT cv.person_id
        FROM current_vals cv
        WHERE cv.person_id != ?
    ",
    );
    for _ in terms {
        sql.push_str(
            " AND EXISTS (SELECT 1 FROM current_vals t \
              WHERE t.person_id = cv.person_id AND t.value_effective LIKE ? ESCAPE '!')",
        );
    }
    sql.push_str(" ORDER BY cv.person_id LIMIT ?");

    let mut values: Vec<sea_orm::Value> = vec![
        tenant_id.as_bytes().to_vec().into(),
        EXCLUDED_PERSON.as_bytes().to_vec().into(),
    ];
    values.extend(terms.iter().map(|t| like_pattern(t).into()));
    values.push(limit.into());

    let stmt = Statement::from_sql_and_values(DbBackend::MySql, &sql, values);
    person_ids_from_rows(db.query_all(stmt).await?)
}

/// `%…%` LIKE pattern for a term, with the wildcard characters neutralised via
/// the `!` escape (declared in the query) — `50%` searches for a literal
/// percent sign instead of everything.
fn like_pattern(term: &str) -> String {
    let escaped = term
        .replace('!', "!!")
        .replace('%', "!%")
        .replace('_', "!_");
    format!("%{escaped}%")
}

/// Hydrate person cards for MANY persons in one query — the shared id→display
/// read behind every operator response that embeds cards (queue candidates,
/// person search). Only the CURRENT observation per person × source × value
/// type leaves the database (the same `rn = 1` window the resolvers use): the
/// journal is append-only, and shipping a person's full re-observation history
/// to collapse it in Rust would grow every response with tenant age. The Rust
/// collapse then picks the latest across sources. An id the journal holds no
/// card attributes for is simply absent from the map, and the caller renders
/// it via [`PersonCard::empty`].
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn person_cards(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_ids: &[Uuid],
) -> anyhow::Result<HashMap<Uuid, PersonCard>> {
    if person_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let id_placeholders = vec!["?"; person_ids.len()].join(", ");
    let type_list = CARD_VALUE_TYPES
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        r"
        SELECT id, value_type, insight_source_type, insight_source_id,
               insight_tenant_id, value_id, value_full_text, value,
               value_effective, value_hash, person_id, author_person_id,
               reason, created_at
        FROM (
            SELECT p.*,
                   ROW_NUMBER() OVER (
                       PARTITION BY person_id, insight_source_type, insight_source_id, value_type
                       ORDER BY created_at DESC, id DESC
                   ) AS rn
            FROM persons p
            WHERE insight_tenant_id = ?
              AND person_id IN ({id_placeholders})
              AND value_type IN ({type_list})
        ) current_rows
        WHERE rn = 1
    "
    );

    let mut values: Vec<sea_orm::Value> = vec![tenant_id.as_bytes().to_vec().into()];
    values.extend(person_ids.iter().map(|id| id.as_bytes().to_vec().into()));

    let stmt = Statement::from_sql_and_values(DbBackend::MySql, &sql, values);
    let rows = persons::Entity::find().from_raw_sql(stmt).all(db).await?;

    Ok(person_card::assemble_cards(rows))
}

/// Fetch every observation row for a person within the tenant (all value types,
/// all sources). The caller collapses them to the current value per attribute.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn fetch_person_observations(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_id: Uuid,
) -> anyhow::Result<Vec<persons::Model>> {
    let rows = persons::Entity::find()
        .filter(persons::Column::InsightTenantId.eq(tenant_id.as_bytes().to_vec()))
        .filter(persons::Column::PersonId.eq(person_id.as_bytes().to_vec()))
        .all(db)
        .await?;
    Ok(rows)
}

/// One current source-native id for a person (repo-level row). The domain maps
/// it to the API `ProfileIdEntry` — the DB layer stays free of API types, the
/// same way `assemble_profile` maps `persons::Model` to the response.
pub struct SourceIdRow {
    pub source_type: String,
    pub source_id: Uuid,
    pub value: String,
}

/// All current source-native ids for one person — one row per source instance
/// (latest `value_type='id'` per (tenant, person, `source_type`, `source_id`)),
/// ordered by source. Ported from `Sql.Profiles.cs::CurrentSourceIdsForPerson`.
///
/// # Errors
///
/// Returns an error if the query fails or a stored `insight_source_id` is not
/// 16 bytes.
pub async fn current_source_ids_for_person(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_id: Uuid,
) -> anyhow::Result<Vec<SourceIdRow>> {
    // Verbatim from Sql.Profiles.cs::CurrentSourceIdsForPerson, `@param` -> `?`.
    const SQL: &str = r"
        WITH ranked AS (
            SELECT
                insight_source_type,
                insight_source_id,
                value_id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, person_id, insight_source_type, insight_source_id, value_type
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE insight_tenant_id = ?
              AND person_id         = ?
              AND value_type        = 'id'
        )
        SELECT insight_source_type, insight_source_id, value_id AS value
        FROM ranked
        WHERE rn = 1
        ORDER BY insight_source_type, insight_source_id
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            tenant_id.as_bytes().to_vec().into(),
            person_id.as_bytes().to_vec().into(),
        ],
    );

    let rows = db.query_all(stmt).await?;
    let mut ids = Vec::with_capacity(rows.len());
    for row in rows {
        let source_type: String = row.try_get("", "insight_source_type")?;
        let source_id_bytes: Vec<u8> = row.try_get("", "insight_source_id")?;
        // `value_type='id'` rows always carry `value_id` in practice; treat a
        // NULL defensively as empty rather than dropping the source instance.
        let value: Option<String> = row.try_get("", "value")?;
        ids.push(SourceIdRow {
            source_type,
            source_id: Uuid::from_slice(&source_id_bytes)?,
            value: value.unwrap_or_default(),
        });
    }
    Ok(ids)
}

/// One current parent edge for a child, scoped to one source instance
/// (repo-level row). Ported from the .NET `OrgChartEdge`.
pub struct OrgChartEdge {
    pub source_type: String,
    pub source_id: Uuid,
    pub parent_person_id: Uuid,
}

/// Current parent edges for one child (`valid_to IS NULL`), across every source
/// instance, ordered by source. The caller filters to the configured
/// `org_chart` source. Ported from `Sql.OrgChart.cs::CurrentParentsForChild`.
///
/// The `parent_person_id IS NOT NULL` filter matches
/// `Sql.OrgChart.cs::CurrentParentsForChild`: the seed writes Path-B
/// root/membership rows with a NULL parent, and a parent edge with no parent is
/// not an edge — skipping it also avoids decoding a NULL into the non-nullable
/// `parent_person_id`.
///
/// # Errors
///
/// Returns an error if the query fails or a stored id column is not 16 bytes.
pub async fn current_parents_for_child(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    child_person_id: Uuid,
) -> anyhow::Result<Vec<OrgChartEdge>> {
    const SQL: &str = r"
        SELECT insight_source_type, insight_source_id, parent_person_id
        FROM org_chart
        WHERE insight_tenant_id = ?
          AND child_person_id   = ?
          AND valid_to IS NULL
          AND parent_person_id IS NOT NULL
        ORDER BY insight_source_type, insight_source_id
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            tenant_id.as_bytes().to_vec().into(),
            child_person_id.as_bytes().to_vec().into(),
        ],
    );

    let rows = db.query_all(stmt).await?;
    let mut edges = Vec::with_capacity(rows.len());
    for row in rows {
        let source_type: String = row.try_get("", "insight_source_type")?;
        let source_id: Vec<u8> = row.try_get("", "insight_source_id")?;
        let parent_person_id: Vec<u8> = row.try_get("", "parent_person_id")?;
        edges.push(OrgChartEdge {
            source_type,
            source_id: Uuid::from_slice(&source_id)?,
            parent_person_id: Uuid::from_slice(&parent_person_id)?,
        });
    }
    Ok(edges)
}

/// One current child edge for a parent (repo-level row). Only the fields the
/// subordinates expansion needs: the source it came from and the child id.
pub struct OrgChartChildEdge {
    pub source_type: String,
    pub child_person_id: Uuid,
}

/// Current direct-children edges for one parent (`valid_to IS NULL`), across
/// every source instance, ordered by source then child. The caller filters to
/// the configured source and de-dupes. Ported from
/// `Sql.OrgChart.cs::CurrentChildrenForParent`.
///
/// # Errors
///
/// Returns an error if the query fails or a stored id column is not 16 bytes.
pub async fn current_children_for_parent(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    parent_person_id: Uuid,
) -> anyhow::Result<Vec<OrgChartChildEdge>> {
    const SQL: &str = r"
        SELECT insight_source_type, child_person_id
        FROM org_chart
        WHERE insight_tenant_id  = ?
          AND parent_person_id   = ?
          AND valid_to IS NULL
        ORDER BY insight_source_type, insight_source_id, child_person_id
    ";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            tenant_id.as_bytes().to_vec().into(),
            parent_person_id.as_bytes().to_vec().into(),
        ],
    );

    let rows = db.query_all(stmt).await?;
    let mut edges = Vec::with_capacity(rows.len());
    for row in rows {
        let source_type: String = row.try_get("", "insight_source_type")?;
        let child_person_id: Vec<u8> = row.try_get("", "child_person_id")?;
        edges.push(OrgChartChildEdge {
            source_type,
            child_person_id: Uuid::from_slice(&child_person_id)?,
        });
    }
    Ok(edges)
}

/// Read the `person_id` (`binary(16)`) column off each result row into a `Uuid`.
fn person_ids_from_rows(rows: Vec<QueryResult>) -> anyhow::Result<Vec<Uuid>> {
    let mut person_ids = Vec::with_capacity(rows.len());
    for row in rows {
        let bytes: Vec<u8> = row.try_get("", "person_id")?;
        person_ids.push(Uuid::from_slice(&bytes)?);
    }
    Ok(person_ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::db;

    #[test]
    fn like_pattern_neutralises_wildcards() {
        for (term, expected) in [
            ("alice", "%alice%"),
            ("50%", "%50!%%"),
            ("a_b", "%a!_b%"),
            ("bang!", "%bang!!%"),
        ] {
            assert_eq!(like_pattern(term), expected, "term: {term:?}");
        }
    }

    /// Integration test against a live MariaDB. Data-dependent (dev cluster).
    /// Set `IDENTITY_TEST_DB_URL` + `IDENTITY_TEST_TENANT_ID` + `IDENTITY_TEST_EMAIL`
    /// (a known email in that tenant) and a MariaDB port-forward to run; skips
    /// cleanly otherwise so CI stays green. The email is not hardcoded so the
    /// test carries no real address and isn't tied to one person.
    #[tokio::test]
    async fn resolve_by_email_against_dev_db() -> anyhow::Result<()> {
        let (Ok(url), Ok(tenant_raw), Ok(known_email)) = (
            std::env::var("IDENTITY_TEST_DB_URL"),
            std::env::var("IDENTITY_TEST_TENANT_ID"),
            std::env::var("IDENTITY_TEST_EMAIL"),
        ) else {
            eprintln!(
                "skip: set IDENTITY_TEST_DB_URL + IDENTITY_TEST_TENANT_ID + IDENTITY_TEST_EMAIL to run"
            );
            return Ok(());
        };
        let tenant = Uuid::parse_str(tenant_raw.trim())?;
        let conn = db::connect(&url).await?;

        let known = resolve_person_ids_by_email(&conn, tenant, known_email.trim()).await?;
        assert_eq!(
            known.len(),
            1,
            "known email should resolve to exactly one person"
        );

        let missing = resolve_person_ids_by_email(&conn, tenant, "nobody@nowhere.invalid").await?;
        assert!(
            missing.is_empty(),
            "unknown email should resolve to zero persons"
        );
        Ok(())
    }
}
