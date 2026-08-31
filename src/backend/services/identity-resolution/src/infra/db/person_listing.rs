//! Ordered, searchable, keyset-paginated reads over the current `people`
//! projection. The projection is the roster; identity observations that have
//! no active roster membership never enter this query.
//!
//! The displayed and ordered label is display name, composed first/last name,
//! username, then email. Search uses the same five projected fields. A bounded
//! probe avoids ranking the whole tenant for selective searches.

use sea_orm::{ConnectionTrait as _, DatabaseConnection, DbBackend, QueryResult, Statement};
use uuid::Uuid;

use crate::config::VisibilityPolicy;

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

/// How many persons the probe may name before the listing stops believing the
/// term is selective. Past this, restricting the ranking costs more than the
/// tenant-wide pass it replaces, so the probe's own LIMIT ends the argument.
const MAX_CANDIDATES: usize = 2_000;

/// One row past the cap — what tells a complete candidate set from a cut one.
const PROBE_LIMIT: u64 = MAX_CANDIDATES as u64 + 1;

/// Whose view narrows a listing. Without a viewer, the whole tenant roster is
/// returned.
#[derive(Debug, Clone, Copy)]
pub struct Restrict<'a> {
    pub visible_to: Option<VisibleTo<'a>>,
}

impl Restrict<'_> {
    #[cfg(test)]
    pub(super) const UNRESTRICTED: Self = Self { visible_to: None };
}

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
/// projected value. `within` (when non-empty) restricts the set to those ids —
/// the id-named search path. `visible_to` restricts it to one caller's visible
/// set. `after` resumes a previous page.
///
/// Two statements when there are terms and no ids: the probe, then the page over
/// what it named. One when there is nothing to narrow by.
///
/// # Errors
///
/// Returns an error if either query fails.
pub async fn list_persons(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    terms: &[String],
    within: &[Uuid],
    restrict: Restrict<'_>,
    after: Option<After<'_>>,
    limit: u64,
) -> anyhow::Result<Vec<PersonListRow>> {
    match narrowing(db, tenant_id, terms, within).await? {
        Narrowing::Nobody => Ok(Vec::new()),
        Narrowing::Tenant => page(db, tenant_id, terms, None, restrict, after, limit).await,
        Narrowing::Persons(ids) => {
            page(db, tenant_id, terms, Some(&ids), restrict, after, limit).await
        }
    }
}

/// What the ranking is allowed to read.
#[derive(Debug)]
enum Narrowing {
    /// Nothing narrows it — the tenant is the set.
    Tenant,
    /// Only these persons can appear. Never empty: that state is `Nobody`.
    Persons(Vec<Uuid>),
    /// The probe named nobody, so no person can pass the filter either and the
    /// page is empty without asking the database a second time.
    Nobody,
}

async fn page(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    terms: &[String],
    narrowed_to: Option<&[Uuid]>,
    restrict: Restrict<'_>,
    after: Option<After<'_>>,
    limit: u64,
) -> anyhow::Result<Vec<PersonListRow>> {
    let (sql, values) = build_query(tenant_id, terms, narrowed_to, restrict, after, limit);
    let stmt = Statement::from_sql_and_values(DbBackend::MySql, &sql, values);
    rows_from(db.query_all_raw(stmt).await?)
}

/// Decide what the ranking may read.
///
/// Named ids answer this by themselves and skip the probe. Otherwise the probe
/// runs, and a set it could not keep under [`MAX_CANDIDATES`] is no narrowing at
/// all: the term reaches too much of the roster to be worth restricting by.
async fn narrowing(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    terms: &[String],
    within: &[Uuid],
) -> anyhow::Result<Narrowing> {
    if !within.is_empty() {
        return Ok(Narrowing::Persons(within.to_vec()));
    }
    let Some((first, rest)) = terms.split_first() else {
        return Ok(Narrowing::Tenant);
    };

    Ok(from_probe(
        candidate_persons(db, tenant_id, first, rest).await?,
    ))
}

/// Read the probe's answer: nobody, a set worth ranking by, or a term so common
/// that the set it collected is neither complete nor worth ranking by.
fn from_probe(candidates: Vec<Uuid>) -> Narrowing {
    if candidates.is_empty() {
        return Narrowing::Nobody;
    }
    if candidates.len() > MAX_CANDIDATES {
        return Narrowing::Tenant;
    }
    Narrowing::Persons(candidates)
}

/// Every person a term could possibly reach, read straight off the raw
/// people projection. One row past the cap is enough to know
/// the term is not selective, so that is where the probe stops.
async fn candidate_persons(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    first: &str,
    rest: &[String],
) -> anyhow::Result<Vec<Uuid>> {
    let (sql, values) = build_probe(tenant_id, first, rest);
    let stmt = Statement::from_sql_and_values(DbBackend::MySql, &sql, values);

    let rows = db.query_all_raw(stmt).await?;
    let mut found = Vec::with_capacity(rows.len());
    for row in rows {
        let raw: Vec<u8> = row.try_get("", "person_id")?;
        found.push(Uuid::from_slice(&raw)?);
    }
    Ok(found)
}

/// The probe statement: the first term narrows, every further one is a lookup
/// per surviving person. Ordering does not matter — the caller only counts and
/// restricts by the set.
fn build_probe(tenant_id: Uuid, first: &str, rest: &[String]) -> (String, Vec<sea_orm::Value>) {
    let mut values: Vec<sea_orm::Value> = vec![tenant_id.as_bytes().to_vec().into()];
    let matches = "(display_name LIKE ? ESCAPE '!' OR first_name LIKE ? ESCAPE '!' \
         OR last_name LIKE ? ESCAPE '!' OR username LIKE ? ESCAPE '!' \
         OR email LIKE ? ESCAPE '!')";
    push_patterns(&mut values, first);
    let also = format!("\n          AND {matches}").repeat(rest.len());
    for term in rest {
        push_patterns(&mut values, term);
    }

    values.push(PROBE_LIMIT.into());

    let sql = format!(
        r"
        SELECT p.person_id
        FROM people p
        WHERE p.insight_tenant_id = ?
          AND p.valid_to IS NULL
          AND {matches}{also}
        LIMIT ?
    "
    );

    (sql, values)
}

/// The listing with the probe deliberately skipped — the shape a term that names
/// more than [`MAX_CANDIDATES`] persons falls back to.
///
/// Test-only: the fallback must answer exactly what the narrowed path answers,
/// and only a live case can say whether it does.
#[cfg(test)]
pub(super) async fn list_persons_unnarrowed(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    terms: &[String],
    after: Option<After<'_>>,
    limit: u64,
) -> anyhow::Result<Vec<PersonListRow>> {
    page(
        db,
        tenant_id,
        terms,
        None,
        Restrict::UNRESTRICTED,
        after,
        limit,
    )
    .await
}

struct VisibleScopeSql {
    /// `RECURSIVE `, so the `WITH` admits the recursive CTE.
    recursive: &'static str,
    cte: String,
    filter: &'static str,
    /// In placeholder order.
    values: Vec<sea_orm::Value>,
}

/// INVARIANT: these values bind after the ranking's and before the filters' —
/// the position the CTE occupies in the statement.
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

fn build_query(
    tenant_id: Uuid,
    terms: &[String],
    narrowed_to: Option<&[Uuid]>,
    restrict: Restrict<'_>,
    after: Option<After<'_>>,
    limit: u64,
) -> (String, Vec<sea_orm::Value>) {
    let mut values = vec![tenant_id.as_bytes().to_vec().into()];
    let within = match narrowed_to {
        None => String::new(),
        Some(ids) => {
            let placeholders = vec!["?"; ids.len()].join(", ");
            values.extend(ids.iter().map(|id| id.as_bytes().to_vec().into()));
            format!("\n              AND person_id IN ({placeholders})")
        }
    };

    let scope = visible_scope(restrict.visible_to, tenant_id);
    let visible_cte = scope.cte;
    let recursive = scope.recursive;
    values.extend(scope.values);

    let mut filters = String::from(scope.filter);
    for term in terms {
        filters.push_str(
            " AND (p.display_name LIKE ? ESCAPE '!' OR p.first_name LIKE ? ESCAPE '!' \
             OR p.last_name LIKE ? ESCAPE '!' OR p.username LIKE ? ESCAPE '!' \
             OR p.email LIKE ? ESCAPE '!')",
        );
        push_patterns(&mut values, term);
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
        WITH {recursive}roster_people AS (
            SELECT person_id, email, username, display_name, first_name, last_name
            FROM people
            WHERE insight_tenant_id = ? AND valid_to IS NULL{within}
        ),
        presented_people AS (
            SELECT person_id, email, username, display_name, first_name, last_name,
                   COALESCE(
                       NULLIF(TRIM(display_name), ''),
                       NULLIF(TRIM(CONCAT_WS(' ',
                           NULLIF(TRIM(first_name), ''),
                           NULLIF(TRIM(last_name), '')
                       )), ''),
                       NULLIF(TRIM(username), ''),
                       NULLIF(TRIM(email), '')
                   ) AS label
            FROM roster_people
        ){visible_cte}
        SELECT person_id, order_key FROM (
            SELECT p.person_id AS person_id,
                   CONCAT(
                       IF(p.label IS NULL, '1', '0'),
                       LEFT(LOWER(COALESCE(p.label, '')), {ORDER_KEY_CHARS})
                   ) AS order_key
            FROM presented_people p
            WHERE 1 = 1{filters}
        ) keyed{resume}
        ORDER BY order_key, person_id
        LIMIT ?
    "
    );

    (sql, values)
}

fn push_patterns(values: &mut Vec<sea_orm::Value>, term: &str) {
    let pattern = like_pattern(term);
    values.extend([
        pattern.clone().into(),
        pattern.clone().into(),
        pattern.clone().into(),
        pattern.clone().into(),
        pattern.into(),
    ]);
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

    fn query(terms: &[String], narrowed_to: Option<&[Uuid]>, after: Option<After<'_>>) -> String {
        build_query(
            Uuid::nil(),
            terms,
            narrowed_to,
            Restrict::UNRESTRICTED,
            after,
            50,
        )
        .0
    }

    fn visible_query(policy: VisibilityPolicy) -> String {
        build_query(
            Uuid::nil(),
            &[],
            None,
            Restrict {
                visible_to: Some(VisibleTo {
                    viewer_person_id: Uuid::from_u128(7),
                    org_source_type: "bamboohr",
                    policy,
                }),
            },
            None,
            50,
        )
        .0
    }

    #[test]
    fn a_query_with_no_terms_filters_nothing_and_still_pages() {
        let sql = query(&[], None, None);

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

        let sql = query(&terms, None, None);

        assert_eq!(
            sql.matches("p.display_name LIKE ?").count(),
            terms.len(),
            "each term must add one match group: {sql}"
        );
        assert!(
            !sql.contains("OR EXISTS"),
            "a person must match both terms, not either: {sql}"
        );
    }

    #[test]
    fn a_narrowed_page_ranks_the_named_persons_and_not_the_tenant() {
        // The whole point of the two-step shape: the window passes see the
        // persons the probe named. A ranking that reads the tenant anyway is the
        // slow query this exists to avoid.
        let ids = [Uuid::from_u128(0xB2), Uuid::from_u128(0xB3)];

        let sql = query(&["iva".to_owned()], Some(&ids), None);

        assert!(
            sql.contains("AND person_id IN (?, ?)"),
            "the roster must be cut to the named persons: {sql}"
        );
    }

    #[test]
    fn an_unnarrowed_page_ranks_the_tenant_without_a_semi_join() {
        // Browsing, and the fallback a term too common to narrow by lands in:
        // there is no set to join against, and adding one would make the plain
        // scan slower for no gain.
        let sql = query(&["iva".to_owned()], None, None);

        assert!(
            !sql.contains("IN (SELECT person_id FROM people)"),
            "an unnarrowed roster must stay a plain scan: {sql}"
        );
        assert!(
            sql.contains("p.display_name LIKE ?"),
            "filter still applies: {sql}"
        );
    }

    #[test]
    fn the_probe_requires_every_term_of_a_person() {
        let terms = ["iva".to_owned(), "example.com".to_owned()];

        let (sql, values) = build_probe(Uuid::nil(), &terms[0], &terms[1..]);

        assert_eq!(
            sql.matches("LIKE ?").count(),
            terms.len() * 5,
            "five projected fields per term: {sql}"
        );
        assert_eq!(
            sql.matches("AND (display_name LIKE ?").count(),
            terms.len(),
            "terms are ANDed: {sql}"
        );
        assert!(
            sql.contains("LIMIT ?"),
            "the probe must stop at the cap: {sql}"
        );
        assert_eq!(
            values.last(),
            Some(&sea_orm::Value::from(PROBE_LIMIT)),
            "the probe reads one row past the cap, so a full set is recognisable"
        );
    }

    #[test]
    fn the_probes_answer_decides_which_shape_runs() {
        let under: Vec<Uuid> = (0..3).map(Uuid::from_u128).collect();
        let over: Vec<Uuid> = (0..=MAX_CANDIDATES as u128).map(Uuid::from_u128).collect();

        assert!(
            matches!(from_probe(under.clone()), Narrowing::Persons(ids) if ids == under),
            "a small set narrows the ranking"
        );
        assert!(
            matches!(from_probe(over), Narrowing::Tenant),
            "a set past the cap falls back to the tenant"
        );
        // Not an empty `IN ()`, and not a statement at all: nobody can match.
        assert!(matches!(from_probe(Vec::new()), Narrowing::Nobody));
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
            Some(&within),
            Restrict::UNRESTRICTED,
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
                bytes(within[0]),
                sea_orm::Value::from("%iva%".to_owned()),
                sea_orm::Value::from("%iva%".to_owned()),
                sea_orm::Value::from("%iva%".to_owned()),
                sea_orm::Value::from("%iva%".to_owned()),
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
            None,
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
        assert!(query(&[], None, None).contains(&format!("), {ORDER_KEY_CHARS})")));
    }

    #[test]
    fn an_unrestricted_listing_asks_nothing_about_visibility() {
        let sql = query(&[], None, None);

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
    fn the_visible_set_values_land_between_the_ranking_and_the_filters() {
        // The CTE sits between them in the SQL, so its values must too.
        let tenant = Uuid::from_u128(0xA1);
        let viewer = Uuid::from_u128(0x7);
        let within = [Uuid::from_u128(0xB2)];

        let (sql, values) = build_query(
            tenant,
            &["iva".to_owned()],
            Some(&within),
            Restrict {
                visible_to: Some(VisibleTo {
                    viewer_person_id: viewer,
                    org_source_type: "bamboohr",
                    policy: VisibilityPolicy::Flat,
                }),
            },
            None,
            50,
        );

        let bytes = |id: Uuid| sea_orm::Value::from(id.as_bytes().to_vec());
        assert_eq!(sql.matches('?').count(), values.len());
        assert_eq!(
            values,
            vec![
                bytes(tenant),
                bytes(within[0]),
                bytes(viewer),
                bytes(tenant),
                bytes(viewer),
                bytes(tenant),
                sea_orm::Value::from(true),
                bytes(tenant),
                bytes(viewer),
                bytes(tenant),
                sea_orm::Value::from("bamboohr"),
                sea_orm::Value::from("%iva%".to_owned()),
                sea_orm::Value::from("%iva%".to_owned()),
                sea_orm::Value::from("%iva%".to_owned()),
                sea_orm::Value::from("%iva%".to_owned()),
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
                None,
                Restrict {
                    visible_to: Some(VisibleTo {
                        viewer_person_id: Uuid::from_u128(7),
                        org_source_type: "bamboohr",
                        policy,
                    }),
                },
                None,
                50,
            );

            assert!(
                values.contains(&sea_orm::Value::from(expected)),
                "{policy:?} must bind {expected}"
            );
        }
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
