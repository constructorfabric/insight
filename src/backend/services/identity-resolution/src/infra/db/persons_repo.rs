//! Read queries against the identity store (`persons`).
//!
//! The resolution queries use window functions (`ROW_NUMBER()` over the canonical partition) that have no
//! first-class SeaORM query-builder form and no `toolkit-db` equivalent (see
//! `infra::db` module docs + constructorfabric/gears-rust#4239), so we run them
//! as **raw SQL** via SeaORM's `Statement` and read columns off the
//! `QueryResult`. Rows naming the excluded-person sentinel (ADR-0003; only
//! the correction verbs mint them) are filtered AFTER the latest-wins ranking,
//! so an excluded account resolves as no person rather than as the shared
//! sentinel, and an older binding is never resurrected past an exclusion.

use std::collections::{HashMap, HashSet};

use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, QueryFilter,
    QueryResult, QuerySelect, Statement,
};
use uuid::Uuid;

use super::entities::persons;
use crate::domain::person_card::{self, CARD_VALUE_TYPES, PersonCard};
use crate::domain::profile::SAFE_PROFILE_ATTRIBUTE_TYPES;
use crate::domain::provenance::UNCONFIRMED_MINT_REASONS;
use crate::domain::resolution::EXCLUDED_PERSON;

#[derive(Debug, thiserror::Error)]
pub enum BatchProfileReadError {
    #[error("batch profile query failed")]
    Database(#[from] sea_orm::DbErr),
    #[error("batch profile row decoding failed: {0}")]
    RowDecode(String),
    #[error("batch profile row contains an invalid person id")]
    InvalidPersonId(#[from] uuid::Error),
}

impl From<sea_orm::TryGetError> for BatchProfileReadError {
    fn from(error: sea_orm::TryGetError) -> Self {
        match error {
            sea_orm::TryGetError::DbErr(error) => Self::Database(error),
            sea_orm::TryGetError::Null(column) => Self::RowDecode(column),
        }
    }
}

/// Resolve the set of `person_id`s whose CURRENT email (latest observation per
/// source instance) equals `email` within the tenant.
///
/// The caller maps the result to the contract: 0 rows → 404 `person_not_found`,
/// 1 → resolved, >1 → 422 `ambiguous_profile`.
///
/// Case handling (ADR-0011): the input is trimmed
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

    let rows = db.query_all_raw(stmt).await?;
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
/// a window `ROW_NUMBER()`
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

    match db.query_one_raw(stmt).await? {
        Some(row) => {
            let bytes: Vec<u8> = row.try_get("", "person_id")?;
            Ok(Some(Uuid::from_slice(&bytes)?))
        }
        None => Ok(None),
    }
}

/// Roster-email → candidate `person_id`s for the login bootstrap of an install
/// that resolves logins by address (`idp.resolve_by = email`,
/// `GET /internal/persons/by-roster-email`).
///
/// A SEPARATE function from `resolve_person_id_by_email_any_tenant` on purpose,
/// even though both match on an address: that one serves the admin `__override`
/// and matches an address stated by ANY source in ANY tenant, which is the right
/// latitude for an operator typing a name and far too much for a sign-in. This
/// one is confined three ways.
///
/// **To the caller's tenant.** Unlike the two any-tenant resolvers, the tenant
/// IS known here: the authenticator denies a login whose `id_token` named
/// no tenant before it ever calls identity, and mints its service JWT with that
/// tenant, so the handler reads it off the `SecurityContext`. An address cannot
/// carry the uniqueness a directory id does — every tenant's roster writes into
/// the same `(source_type, address)` key space, and one customer adding another
/// customer's address to its own HR system would otherwise resolve that
/// customer's login to a person of its choosing.
///
/// **To the source the install declares as its roster** (`roster_source_type`),
/// so an address only a chat or an issue tracker ever observed admits nobody.
///
/// **To a person the roster still holds a live account for.** Exclusion
/// (ADR-0003) is recorded only against an ACCOUNT, as a `value_type='id'` row
/// naming the sentinel, and the seed then stops re-emitting that account's
/// values — so the address row written before the exclusion stays newest
/// forever and would keep resolving the person it named. Filtering the sentinel
/// out of THIS query cannot help: no `email` row ever names it. The live-binding
/// requirement is what makes an exclusion bite at the door, and it also answers
/// "may an address observed with no account behind it admit anyone" with no.
///
/// Returns every distinct candidate, newest observation first. More than one
/// means the roster states one address for two people — the seed refuses to
/// auto-link that shape (`skipped_contested_email`) and an operator may have
/// split them deliberately, so the caller decides what to do rather than being
/// handed a silent winner.
///
/// Case handling matches the rest of the resolvers: the input is trimmed only,
/// and `value_id`'s collation does case-insensitive matching (migration 004).
///
/// **To the address the roster states now.** `persons` is append-only, so a
/// person keeps every address they were ever observed under. Matching any of
/// them would mean a leaver's alias, handed to a new hire, still signs the new
/// hire in as the leaver. Only the newest address observation per roster
/// account counts.
///
/// # Errors
///
/// Returns an error if the query fails or a stored `person_id` is not 16 bytes.
pub async fn resolve_person_ids_by_roster_email(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    roster_source_type: &str,
    email: &str,
) -> anyhow::Result<Vec<Uuid>> {
    const SQL: &str = r"
        WITH addressed AS (
            SELECT DISTINCT insight_source_id, person_id
            FROM persons
            WHERE value_type = 'email'
              AND insight_tenant_id = ?
              AND insight_source_type = ?
              AND value_id = ?
        ),
        current_address AS (
            SELECT
                p.person_id,
                p.value_id,
                p.created_at,
                p.id,
                ROW_NUMBER() OVER (
                    PARTITION BY p.insight_source_id, p.person_id
                    ORDER BY p.created_at DESC, p.id DESC
                ) AS rn
            FROM persons p
            JOIN addressed a
              ON a.insight_source_id = p.insight_source_id
             AND a.person_id = p.person_id
            WHERE p.value_type = 'email'
              AND p.insight_tenant_id = ?
              AND p.insight_source_type = ?
        ),
        bindings AS (
            SELECT
                person_id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_source_id, value_id
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE value_type = 'id'
              AND insight_tenant_id = ?
              AND insight_source_type = ?
        )
        SELECT c.person_id
        FROM current_address c
        WHERE c.rn = 1
          AND c.value_id = ?
          AND c.person_id != ?
          AND EXISTS (
              SELECT 1 FROM bindings b
              WHERE b.rn = 1 AND b.person_id = c.person_id
          )
        GROUP BY c.person_id
        ORDER BY MAX(c.created_at) DESC, MAX(c.id) DESC
    ";

    let tenant_bytes = tenant_id.as_bytes().to_vec();
    let source = roster_source_type.trim().to_owned();
    let address = email.trim().to_owned();
    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            // addressed: who ever stated it (index-friendly, narrows the rest)
            tenant_bytes.clone().into(),
            source.clone().into(),
            address.clone().into(),
            // current_address: their newest address, for just those pairs
            tenant_bytes.clone().into(),
            source.clone().into(),
            // bindings: accounts the roster currently holds
            tenant_bytes.into(),
            source.into(),
            // and the newest address must still be the one presented
            address.into(),
            EXCLUDED_PERSON.as_bytes().to_vec().into(),
        ],
    );

    person_ids_from_rows(db.query_all_raw(stmt).await?)
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

    match db.query_one_raw(stmt).await? {
        Some(row) => {
            let bytes: Vec<u8> = row.try_get("", "person_id")?;
            Ok(Some(Uuid::from_slice(&bytes)?))
        }
        None => Ok(None),
    }
}

/// Resolve the set of `person_id`s whose CURRENT `value_type='id'` observation
/// on the given source instance (`source_type` + `source_id`) equals `value`.
/// Source-instance scoped.
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
            // Source-native ids are matched as-is; only the email path trims.
            value.to_owned().into(),
            EXCLUDED_PERSON.as_bytes().to_vec().into(),
        ],
    );

    let rows = db.query_all_raw(stmt).await?;
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
) -> Result<Vec<Uuid>, BatchProfileReadError> {
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

/// Of the given persons, those the journal holds nothing but automatic mints for
/// — a sign-in that needed somebody to enter as, or a roster that listed an
/// account carrying no address.
///
/// Either way nobody has decided who they are, so they may well duplicate a
/// person the roster already knows. Naming one as a merge target is the wrong
/// direction: the history is on the other side.
///
/// # Errors
///
/// Returns an error if the query fails or a stored `person_id` is not 16 bytes.
pub async fn provisional_persons(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_ids: &[Uuid],
) -> anyhow::Result<HashSet<Uuid>> {
    if person_ids.is_empty() {
        return Ok(HashSet::new());
    }

    let placeholders = vec!["?"; person_ids.len()].join(", ");
    // SAFETY: `<=>`, not `=` — migration 009 made `reason` nullable, and `=`
    // would make the CASE NULL for such a row, the SUM NULL with it, and drop
    // the person from the answer. A NULL reason is "not an automatic mint".
    let mint_reasons = vec!["reason <=> ?"; UNCONFIRMED_MINT_REASONS.len()].join(" OR ");
    let sql = format!(
        "SELECT person_id          FROM persons          WHERE insight_tenant_id = ? AND person_id IN ({placeholders})          GROUP BY person_id          HAVING SUM(CASE WHEN {mint_reasons} THEN 0 ELSE 1 END) = 0"
    );

    let mut params: Vec<sea_orm::Value> =
        Vec::with_capacity(person_ids.len() + 1 + UNCONFIRMED_MINT_REASONS.len());
    params.push(tenant_id.as_bytes().to_vec().into());
    for id in person_ids {
        params.push(id.as_bytes().to_vec().into());
    }
    for reason in UNCONFIRMED_MINT_REASONS {
        params.push(reason.into());
    }

    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            &sql,
            params,
        ))
        .await?;

    let mut provisional = HashSet::with_capacity(rows.len());
    for row in rows {
        let person_id: Vec<u8> = row.try_get("", "person_id")?;
        provisional.insert(Uuid::from_slice(&person_id)?);
    }
    Ok(provisional)
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
               value_effective, person_id, author_person_id,
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

pub async fn current_profile_observations(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    person_ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<persons::Model>>, BatchProfileReadError> {
    if person_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let id_placeholders = vec!["?"; person_ids.len()].join(", ");
    let type_list = SAFE_PROFILE_ATTRIBUTE_TYPES
        .iter()
        .map(|value_type| format!("'{value_type}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        r"
        SELECT id, value_type, insight_source_type, insight_source_id,
               insight_tenant_id, value_id, value_full_text, value,
               value_effective, person_id, author_person_id,
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
    let mut observations = HashMap::new();
    for row in rows {
        observations
            .entry(Uuid::from_slice(&row.person_id)?)
            .or_insert_with(Vec::new)
            .push(row);
    }
    Ok(observations)
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
/// ordered by source.
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

    let rows = db.query_all_raw(stmt).await?;
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
/// (repo-level row).
pub struct OrgChartEdge {
    pub source_type: String,
    pub source_id: Uuid,
    pub parent_person_id: Uuid,
}

pub struct OrgChartParentEdge {
    pub child_person_id: Uuid,
    pub source_type: String,
    pub parent_person_id: Uuid,
}

/// Current parent edges for one child (`valid_to IS NULL`), across every source
/// instance, ordered by source. The caller filters to the configured
/// `org_chart` source.
///
/// The `parent_person_id IS NOT NULL` filter is deliberate: the seed writes Path-B
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

    let rows = db.query_all_raw(stmt).await?;
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

pub async fn current_parents_for_children(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    child_person_ids: &[Uuid],
) -> Result<Vec<OrgChartParentEdge>, BatchProfileReadError> {
    if child_person_ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders = vec!["?"; child_person_ids.len()].join(", ");
    let sql = format!(
        r"
        SELECT child_person_id, insight_source_type, parent_person_id
        FROM org_chart
        WHERE insight_tenant_id = ?
          AND child_person_id IN ({placeholders})
          AND valid_to IS NULL
          AND parent_person_id IS NOT NULL
        ORDER BY child_person_id, insight_source_type, insight_source_id
    "
    );

    let mut values: Vec<sea_orm::Value> = vec![tenant_id.as_bytes().to_vec().into()];
    values.extend(
        child_person_ids
            .iter()
            .map(|person_id| person_id.as_bytes().to_vec().into()),
    );

    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            &sql,
            values,
        ))
        .await?;
    let mut edges = Vec::with_capacity(rows.len());
    for row in rows {
        let child_person_id: Vec<u8> = row.try_get("", "child_person_id")?;
        let source_type: String = row.try_get("", "insight_source_type")?;
        let parent_person_id: Vec<u8> = row.try_get("", "parent_person_id")?;
        edges.push(OrgChartParentEdge {
            child_person_id: Uuid::from_slice(&child_person_id)?,
            source_type,
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
/// the configured source and de-dupes.
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

    let rows = db.query_all_raw(stmt).await?;
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
