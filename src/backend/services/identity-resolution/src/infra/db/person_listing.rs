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

use crate::config::VisibilityPolicy;
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

/// Restrict a listing to what one caller may see. Absent, it is the tenant's.
#[derive(Debug, Clone, Copy)]
pub struct VisibleTo<'a> {
    pub viewer_person_id: Uuid,
    pub org_source_type: &'a str,
    pub policy: VisibilityPolicy,
}

/// INVARIANT: the same union `subchart_repo` evaluates. A second rule here
/// would let a listing show a person the batch filter refuses to confirm.
const VISIBLE_SET_CTE: &str = r"
        visible_set (person_id) AS (
            SELECT ?
            UNION
            SELECT viewed_person_id
            FROM visibility
            WHERE insight_tenant_id = ?
              AND viewer_person_id  = ?
              AND viewed_person_id  IS NOT NULL
              AND valid_from <= UTC_TIMESTAMP(6)
              AND (valid_to IS NULL OR valid_to > UTC_TIMESTAMP(6))
            UNION
            SELECT DISTINCT person_id
            FROM persons
            WHERE insight_tenant_id = ?
              AND (? OR EXISTS (
                  SELECT 1 FROM visibility
                  WHERE insight_tenant_id = ?
                    AND viewer_person_id  = ?
                    AND viewed_person_id  IS NULL
                    AND valid_from <= UTC_TIMESTAMP(6)
                    AND (valid_to IS NULL OR valid_to > UTC_TIMESTAMP(6))
              ))
            UNION
            SELECT oc.child_person_id
            FROM visible_set vs
            JOIN org_chart oc
              ON  oc.parent_person_id    = vs.person_id
              AND oc.insight_tenant_id   = ?
              AND oc.insight_source_type = ?
              AND oc.valid_from <= UTC_TIMESTAMP(6)
              AND (oc.valid_to IS NULL OR oc.valid_to > UTC_TIMESTAMP(6))
        )";

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
/// the id-named search path. `visible_to` restricts it to one caller's visible
/// set. `after` resumes a previous page.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn list_persons(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    terms: &[String],
    within: &[Uuid],
    visible_to: Option<VisibleTo<'_>>,
    after: Option<After<'_>>,
    limit: u64,
) -> anyhow::Result<Vec<PersonListRow>> {
    let (sql, values) = build_query(tenant_id, terms, within, visible_to, after, limit);
    let stmt = Statement::from_sql_and_values(DbBackend::MySql, &sql, values);
    rows_from(db.query_all(stmt).await?)
}

struct VisibleScopeSql {
    /// `RECURSIVE `, so the `WITH` admits the recursive CTE.
    recursive: &'static str,
    cte: String,
    filter: &'static str,
    /// In placeholder order.
    values: Vec<sea_orm::Value>,
}

/// INVARIANT: these values bind between `people`'s and the filters' — the
/// position the CTE occupies in the statement.
fn visible_scope(visible_to: Option<VisibleTo<'_>>, tenant_id: Uuid) -> VisibleScopeSql {
    let Some(scope) = visible_to else {
        return VisibleScopeSql {
            recursive: "",
            cte: String::new(),
            filter: "",
            values: Vec::new(),
        };
    };

    let tenant = || sea_orm::Value::from(tenant_id.as_bytes().to_vec());
    let viewer = || sea_orm::Value::from(scope.viewer_person_id.as_bytes().to_vec());

    VisibleScopeSql {
        recursive: "RECURSIVE ",
        cte: format!(",\n{VISIBLE_SET_CTE}"),
        filter: " AND EXISTS (SELECT 1 FROM visible_set vs WHERE vs.person_id = p.person_id)",
        values: vec![
            viewer(),
            tenant(),
            viewer(),
            tenant(),
            scope.policy.is_flat().into(),
            tenant(),
            viewer(),
            tenant(),
            scope.org_source_type.into(),
        ],
    }
}

struct StatementParts<'a> {
    recursive: &'a str,
    type_list: &'a str,
    visible_cte: &'a str,
    filters: &'a str,
    resume: &'a str,
}

/// The statement text, apart from the binding order that can silently drift.
fn statement_sql(parts: &StatementParts<'_>) -> String {
    let StatementParts {
        recursive,
        type_list,
        visible_cte,
        filters,
        resume,
    } = *parts;
    format!(
        r"
        WITH {recursive}ranked AS (
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
        ){visible_cte}
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
    )
}

fn build_query(
    tenant_id: Uuid,
    terms: &[String],
    within: &[Uuid],
    visible_to: Option<VisibleTo<'_>>,
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
    // tenant + excluded sentinel, the visible-set CTE, then the filters.
    values.push(tenant_id.as_bytes().to_vec().into());
    values.push(tenant_id.as_bytes().to_vec().into());
    values.push(EXCLUDED_PERSON.as_bytes().to_vec().into());

    let scope = visible_scope(visible_to, tenant_id);
    values.extend(scope.values);

    let mut filters = String::from(scope.filter);
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

    let sql = statement_sql(&StatementParts {
        recursive: scope.recursive,
        type_list: &type_list,
        visible_cte: &scope.cte,
        filters: &filters,
        resume: &resume,
    });

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
        build_query(Uuid::nil(), terms, within, None, after, 50).0
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
            None,
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

    fn visible_query(policy: VisibilityPolicy) -> String {
        build_query(
            Uuid::nil(),
            &[],
            &[],
            Some(VisibleTo {
                viewer_person_id: Uuid::from_u128(7),
                org_source_type: "bamboohr",
                policy,
            }),
            None,
            50,
        )
        .0
    }

    #[test]
    fn an_unrestricted_listing_asks_nothing_about_visibility() {
        let sql = query(&[], &[], None);

        assert!(
            !sql.contains("visible_set"),
            "the operator surface is unfiltered"
        );
        assert!(
            !sql.contains("RECURSIVE"),
            "no recursion without a visible set"
        );
    }

    #[test]
    fn a_restricted_listing_narrows_the_set_to_the_visible_one() {
        let sql = visible_query(VisibilityPolicy::OrgChart);

        assert!(
            sql.contains("WITH RECURSIVE"),
            "the visible set recurses: {sql}"
        );
        assert!(sql.contains("visible_set"));
        assert!(
            sql.contains("EXISTS (SELECT 1 FROM visible_set"),
            "the restriction is a filter over the listed people: {sql}"
        );
    }

    #[test]
    fn the_visible_set_values_land_between_the_people_and_the_filters() {
        // The CTE sits between them in the SQL, so its values must too.
        let tenant = Uuid::from_u128(0xA1);
        let viewer = Uuid::from_u128(0x7);
        let within = [Uuid::from_u128(0xB2)];

        let (sql, values) = build_query(
            tenant,
            &["iva".to_owned()],
            &within,
            Some(VisibleTo {
                viewer_person_id: viewer,
                org_source_type: "bamboohr",
                policy: VisibilityPolicy::Flat,
            }),
            None,
            50,
        );

        let bytes = |id: Uuid| sea_orm::Value::from(id.as_bytes().to_vec());
        assert_eq!(sql.matches('?').count(), values.len());
        assert_eq!(
            values,
            vec![
                bytes(tenant),
                bytes(tenant),
                bytes(EXCLUDED_PERSON),
                // the visible-set union
                bytes(viewer),
                bytes(tenant),
                bytes(viewer),
                bytes(tenant),
                sea_orm::Value::from(true),
                bytes(tenant),
                bytes(viewer),
                bytes(tenant),
                sea_orm::Value::from("bamboohr"),
                // then the filters it narrows
                bytes(within[0]),
                sea_orm::Value::from("%iva%".to_owned()),
                sea_orm::Value::from(50u64),
            ],
            "the bind order drifted from the order the placeholders appear in"
        );
    }

    #[test]
    fn the_policy_reaches_the_visible_set_as_a_bound_flag() {
        for (policy, expected) in [
            (VisibilityPolicy::OrgChart, false),
            (VisibilityPolicy::Flat, true),
        ] {
            let (_, values) = build_query(
                Uuid::nil(),
                &[],
                &[],
                Some(VisibleTo {
                    viewer_person_id: Uuid::from_u128(7),
                    org_source_type: "bamboohr",
                    policy,
                }),
                None,
                50,
            );

            assert!(
                values.contains(&sea_orm::Value::from(expected)),
                "{policy:?} must bind {expected}"
            );
        }
    }
}
