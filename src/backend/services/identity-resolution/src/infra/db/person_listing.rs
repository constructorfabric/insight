//! The operator's person listing — one ordered, keyset-paginated query behind
//! both browsing every person and searching for one.
//!
//! Browse and search are the same set with a different filter, so they are one
//! query: an operator narrowing the terms must not get a differently ordered
//! list, and a page boundary must fall in the same place either way.
//!
//! **The order is the card's own label** — display name, else the composed
//! first/last, else the address, else the source-native handle — folded to
//! lower case, with the people the journal knows by nothing but an id last.
//! Ordering by anything else would sort the list by a value the rows do not
//! show. The label is computed from the same latest-observation-wins rule
//! `person_card` assembles cards by, so the two cannot disagree.
//!
//! The filter deliberately matches a WIDER surface than the order: every value
//! that is current **for its own source**, not just the globally latest one.
//! A person renamed in one system stays findable by the name another system
//! still reports, which is exactly what an operator arrives holding.
//!
//! INVARIANT: this is a deliberate tenant-bounded scan, not a missing index.
//! A substring match over the journal cannot use a B-tree, and a derived
//! current-values table would be one more cache to keep in sync with every
//! write path. The consumer is the admin console — one operator at a time —
//! and the scan is bounded by the tenant filter, the six searchable value
//! types and the page. Revisit as a derived table only if measured slow at
//! real scale.

use sea_orm::{ConnectionTrait as _, DatabaseConnection, DbBackend, QueryResult, Statement};
use uuid::Uuid;

use crate::domain::resolution::EXCLUDED_PERSON;

/// Observation attributes a term may match. Wider than the label's inputs:
/// `employee_id` is worth searching by and worth nothing to sort by.
const FILTERABLE_VALUE_TYPES: [&str; 6] = [
    "email",
    "username",
    "display_name",
    "first_name",
    "last_name",
    "employee_id",
];

/// One listed person and the position that orders them.
#[derive(Debug, Clone)]
pub struct PersonListRow {
    pub person_id: Uuid,
    /// Opaque ordering position — the value the page cursor resumes after.
    /// Not for display: it carries the unnamed-last band as a leading digit.
    pub order_key: String,
}

/// Where a page resumes: the `(order_key, person_id)` of the last row served.
#[derive(Debug, Clone, Copy)]
pub struct After<'a> {
    pub order_key: &'a str,
    pub person_id: Uuid,
}

/// One page of the tenant's persons, ordered by their card label.
///
/// `terms` empty lists every person; otherwise every term must match some
/// current value. `within` (when non-empty) restricts the set to those ids —
/// the id-named search path. `after` resumes a previous page.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn list_persons(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    terms: &[String],
    within: &[Uuid],
    after: Option<After<'_>>,
    limit: u64,
) -> anyhow::Result<Vec<PersonListRow>> {
    let (sql, values) = build_query(tenant_id, terms, within, after, limit);
    let stmt = Statement::from_sql_and_values(DbBackend::MySql, &sql, values);
    rows_from(db.query_all(stmt).await?)
}

fn build_query(
    tenant_id: Uuid,
    terms: &[String],
    within: &[Uuid],
    after: Option<After<'_>>,
    limit: u64,
) -> (String, Vec<sea_orm::Value>) {
    let mut values: Vec<sea_orm::Value> = Vec::new();

    let type_list = FILTERABLE_VALUE_TYPES
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(", ");

    // Bound in the order the placeholders appear: ranked's tenant, people's
    // tenant + excluded sentinel, then the filters.
    values.push(tenant_id.as_bytes().to_vec().into());
    values.push(tenant_id.as_bytes().to_vec().into());
    values.push(EXCLUDED_PERSON.as_bytes().to_vec().into());

    let mut filters = String::new();
    if !within.is_empty() {
        let placeholders = vec!["?"; within.len()].join(", ");
        filters.push_str(" AND p.person_id IN (");
        filters.push_str(&placeholders);
        filters.push(')');
        values.extend(within.iter().map(|id| id.as_bytes().to_vec().into()));
    }
    for term in terms {
        filters.push_str(
            " AND EXISTS (SELECT 1 FROM current_vals t \
             WHERE t.person_id = p.person_id AND t.value_effective LIKE ? ESCAPE '!')",
        );
        values.push(like_pattern(term).into());
    }

    let mut resume = String::new();
    if let Some(after) = after {
        resume.push_str(" WHERE (order_key > ? OR (order_key = ? AND person_id > ?))");
        values.push(after.order_key.into());
        values.push(after.order_key.into());
        values.push(after.person_id.as_bytes().to_vec().into());
    }

    values.push(limit.into());

    let sql = format!(
        r"
        WITH ranked AS (
            SELECT person_id, value_type, value_effective, created_at, id,
                   ROW_NUMBER() OVER (
                       PARTITION BY person_id, insight_source_type, insight_source_id, value_type
                       ORDER BY created_at DESC, id DESC
                   ) AS rn
            FROM persons
            WHERE insight_tenant_id = ?
              AND value_type IN ({type_list})
        ),
        current_vals AS (
            SELECT person_id, value_type, value_effective, created_at, id
            FROM ranked
            WHERE rn = 1 AND value_effective IS NOT NULL
        ),
        latest_vals AS (
            SELECT person_id, value_type, value_effective,
                   ROW_NUMBER() OVER (
                       PARTITION BY person_id, value_type
                       ORDER BY created_at DESC, id DESC
                   ) AS rn
            FROM current_vals
        ),
        card AS (
            SELECT person_id,
                   COALESCE(
                       NULLIF(TRIM(display_name), ''),
                       NULLIF(TRIM(CONCAT_WS(' ',
                           NULLIF(TRIM(first_name), ''),
                           NULLIF(TRIM(last_name), ''))), ''),
                       NULLIF(TRIM(email), ''),
                       NULLIF(TRIM(username), '')
                   ) AS label
            FROM (
                SELECT person_id,
                       MAX(CASE WHEN value_type = 'display_name' THEN value_effective END) AS display_name,
                       MAX(CASE WHEN value_type = 'first_name'   THEN value_effective END) AS first_name,
                       MAX(CASE WHEN value_type = 'last_name'    THEN value_effective END) AS last_name,
                       MAX(CASE WHEN value_type = 'email'        THEN value_effective END) AS email,
                       MAX(CASE WHEN value_type = 'username'     THEN value_effective END) AS username
                FROM latest_vals
                WHERE rn = 1
                GROUP BY person_id
            ) pivoted
        ),
        people AS (
            SELECT DISTINCT person_id
            FROM persons
            WHERE insight_tenant_id = ? AND person_id != ?
        )
        SELECT person_id, order_key FROM (
            SELECT p.person_id AS person_id,
                   CONCAT(
                       IF(c.label IS NULL, '1', '0'),
                       LOWER(COALESCE(c.label, ''))
                   ) AS order_key
            FROM people p
            LEFT JOIN card c ON c.person_id = p.person_id
            WHERE 1 = 1{filters}
        ) keyed{resume}
        ORDER BY order_key, person_id
        LIMIT ?
    "
    );

    (sql, values)
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

fn rows_from(rows: Vec<QueryResult>) -> anyhow::Result<Vec<PersonListRow>> {
    let mut listed = Vec::with_capacity(rows.len());
    for row in rows {
        let bytes: Vec<u8> = row.try_get("", "person_id")?;
        listed.push(PersonListRow {
            person_id: Uuid::from_slice(&bytes)?,
            order_key: row.try_get("", "order_key")?,
        });
    }
    Ok(listed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(terms: &[String], within: &[Uuid], after: Option<After<'_>>) -> String {
        build_query(Uuid::nil(), terms, within, after, 50).0
    }

    #[test]
    fn browsing_binds_no_filter_and_still_orders_and_limits() {
        let sql = query(&[], &[], None);

        assert!(!sql.contains("EXISTS"), "no term filter when browsing");
        assert!(!sql.contains("order_key >"), "no resume on a first page");
        assert!(sql.contains("ORDER BY order_key, person_id"));
        assert!(sql.contains("LIMIT ?"));
    }

    #[test]
    fn every_term_becomes_its_own_match_requirement() {
        let terms = ["iva".to_owned(), "example.com".to_owned()];

        let sql = query(&terms, &[], None);

        assert_eq!(
            sql.matches("EXISTS").count(),
            2,
            "a person must match both terms, not either"
        );
    }

    #[test]
    fn the_listing_starts_from_every_person_not_only_the_described_ones() {
        // A person minted at first sign-in carries no card attributes at all;
        // browsing must still list them, so the base set is the journal's
        // persons and the label join is an outer one.
        let sql = query(&[], &[], None);

        assert!(sql.contains("SELECT DISTINCT person_id"));
        assert!(sql.contains("LEFT JOIN card"));
    }

    #[test]
    fn resuming_compares_the_key_then_breaks_the_tie_on_the_id() {
        // A case-folding collation makes distinct labels compare equal, so the
        // id tie-break is what keeps a page boundary from repeating or
        // skipping those rows.
        let sql = query(
            &[],
            &[],
            Some(After {
                order_key: "0ivanov",
                person_id: Uuid::nil(),
            }),
        );

        assert!(sql.contains("order_key > ? OR (order_key = ? AND person_id > ?)"));
    }

    #[test]
    fn bound_values_follow_the_placeholder_order() {
        let terms = ["iva".to_owned()];
        let within = [Uuid::nil()];

        let (sql, values) = build_query(
            Uuid::nil(),
            &terms,
            &within,
            Some(After {
                order_key: "0ivanov",
                person_id: Uuid::nil(),
            }),
            50,
        );

        assert_eq!(
            sql.matches('?').count(),
            values.len(),
            "a placeholder without its value shifts every later binding"
        );
    }

    #[test]
    fn the_unnamed_band_sorts_after_every_label() {
        // The band is a leading digit on the key rather than a second ORDER BY
        // column, so the cursor stays a single comparable string.
        let sql = query(&[], &[], None);

        assert!(sql.contains("IF(c.label IS NULL, '1', '0')"));
    }

    #[test]
    fn like_pattern_neutralises_wildcards() {
        for (term, expected) in [
            ("alice", "%alice%"),
            ("50%", "%50!%%"),
            ("a_b", "%a!_b%"),
            ("!", "%!!%"),
        ] {
            assert_eq!(like_pattern(term), expected, "should escape: {term:?}");
        }
    }
}
