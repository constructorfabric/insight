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
//! show.
//!
//! INVARIANT: the label follows `person_card`'s rule exactly, including where
//! that rule leaves an attribute absent. Both rank every observation of a value
//! type and only then discard a blank winner — dropping blanks first would
//! promote a value the card does not show, and the list would sort a row under a
//! name nobody sees.
//!
//! The filter deliberately matches a WIDER surface than the order: every value
//! that is current **for its own source**, not just the globally latest one.
//! A person renamed in one system stays findable by the name another system
//! still reports, which is exactly what an operator arrives holding.
//!
//! This is a deliberate tenant-wide scan, not a missing index: a substring match
//! over the journal cannot use a B-tree, and a derived current-values table
//! would be one more cache to keep in sync with every write path. The page
//! bounds what is returned and not the work — every page ranks the tenant's
//! observations again — so the consumer is the admin console, one operator at a
//! time. A derived table is the answer if that stops being true.

use sea_orm::{ConnectionTrait as _, DatabaseConnection, DbBackend, QueryResult, Statement};
use uuid::Uuid;

use crate::domain::resolution::EXCLUDED_PERSON;

/// How much of the label the order key carries.
///
/// INVARIANT: must keep the key under `max_sort_length` (1024 bytes by default,
/// 4 bytes per utf8mb4 character, plus one for the band digit). MariaDB
/// truncates a TEXT sort key at that length but compares the resume predicate in
/// full, so a longer key would order two rows equal and resume between them —
/// and the row on the wrong side of that boundary is unreachable by paging.
/// Truncation only widens the tie the `person_id` tie-break already settles.
const ORDER_KEY_CHARS: usize = 255;

// The bound is only a bound if it fits: four bytes per utf8mb4 character, plus
// the band digit, inside MariaDB's default `max_sort_length`.
const _: () = assert!(ORDER_KEY_CHARS * 4 < 1024);

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
            WHERE rn = 1
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
                       LEFT(LOWER(COALESCE(c.label, '')), {ORDER_KEY_CHARS})
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
    fn a_query_with_no_terms_filters_nothing_and_still_pages() {
        let sql = query(&[], &[], None);

        assert!(!sql.contains("EXISTS"), "no term filter when browsing");
        assert!(!sql.contains("order_key >"), "no resume on a first page");
        assert!(sql.contains("ORDER BY order_key, person_id"));
        assert!(sql.contains("LIMIT ?"));
    }

    #[test]
    fn every_term_becomes_its_own_match_requirement() {
        // ANDed, not ORed: "iva example.com" means a person matching both, and
        // the AND is what the count of clauses cannot see.
        let terms = ["iva".to_owned(), "example.com".to_owned()];

        let sql = query(&terms, &[], None);

        assert_eq!(sql.matches("EXISTS").count(), 2);
        assert!(
            !sql.contains("OR EXISTS"),
            "a person must match both terms, not either: {sql}"
        );
    }

    #[test]
    fn every_value_lands_in_the_slot_its_placeholder_holds() {
        // The bind list is assembled by hand, so a mismatch does not fail — it
        // shifts, and a resume value lands in a LIKE pattern. Counting is not
        // enough; each value has to be recognisable in its own position.
        let terms = ["iva".to_owned()];
        let tenant = Uuid::from_u128(0xA1);
        let within = [Uuid::from_u128(0xB2)];
        let resuming = Uuid::from_u128(0xC3);

        let (sql, values) = build_query(
            tenant,
            &terms,
            &within,
            Some(After {
                order_key: "0ivanov",
                person_id: resuming,
            }),
            50,
        );

        assert_eq!(
            sql.matches('?').count(),
            values.len(),
            "a placeholder without its value shifts every later binding"
        );
        let bytes = |id: Uuid| sea_orm::Value::from(id.as_bytes().to_vec());
        assert_eq!(
            values,
            vec![
                bytes(tenant),
                bytes(tenant),
                bytes(EXCLUDED_PERSON),
                bytes(within[0]),
                sea_orm::Value::from("%iva%".to_owned()),
                sea_orm::Value::from("0ivanov".to_owned()),
                sea_orm::Value::from("0ivanov".to_owned()),
                bytes(resuming),
                sea_orm::Value::from(50u64),
            ],
            "the bind order drifted from the order the placeholders appear in"
        );
    }

    #[test]
    fn resuming_compares_the_key_then_breaks_the_tie_on_the_id() {
        // A case-folding collation makes distinct labels compare equal, and the
        // key is truncated for the sort, which widens that tie further — so the
        // id tie-break is what keeps a page boundary from repeating or skipping
        // those rows.
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
    fn the_order_key_is_cut_to_what_the_sort_can_compare() {
        // MariaDB truncates a TEXT sort key at `max_sort_length` but compares the
        // resume predicate in full. Cutting the label to a length that fits is
        // what stops a long-label boundary from skipping a person for good; the
        // budget itself is checked at compile time beside the constant.
        assert!(query(&[], &[], None).contains(&format!("), {ORDER_KEY_CHARS})")));
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
