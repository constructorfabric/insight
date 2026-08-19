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
//! A substring match over the journal cannot use a B-tree, and the ranking is
//! the expensive half — two window passes over every observation it is handed.
//! So a term is answered in two steps: a probe over the raw rows names the
//! persons it could possibly reach, and the ranking is restricted to those.
//!
//! INVARIANT: the probe must stay a SUPERSET of the exact filter. It reads raw
//! observations where the filter reads current ones, so it can only ever admit
//! too many, and the filter below still decides who is returned. Narrowing the
//! probe to current values would drop rows the filter would have kept — a wrong
//! answer rather than a slow one.
//!
//! A term half the roster matches is not narrowed by anything, so the probe
//! stops at [`MAX_CANDIDATES`] and the ranking reads the tenant instead. Its own
//! LIMIT makes finding that out nearly free, which is what keeps the two-step
//! shape from costing more than it saves. Note what reaches the cap: the probe
//! counts persons a SUPERSEDED value matched too, so a long-lived journal gets
//! there on accumulated history and not only on how common the term is. That
//! costs a fallback, never a wrong answer.
//!
//! An id-named search skips the probe entirely: the ids ARE the set, and the
//! tightest one there is.
//!
//! A listing restricted to one caller's visible set narrows the ranking too, but
//! never the probe: the probe only has to stay a superset, and a visibility
//! filter it does not carry can only leave it wider than it needs to be.
//!
//! Browsing with no terms has nothing to narrow by and ranks the whole tenant —
//! the page bounds what is returned, not the work. That is the one case a
//! derived current-values table would fix, and it is a cache to keep in sync
//! with every write path, so it waits until the roster outgrows one operator.

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

/// How many persons the probe may name before the listing stops believing the
/// term is selective. Past this, restricting the ranking costs more than the
/// tenant-wide pass it replaces, so the probe's own LIMIT ends the argument.
const MAX_CANDIDATES: usize = 2_000;

/// One row past the cap — what tells a complete candidate set from a cut one.
const PROBE_LIMIT: u64 = MAX_CANDIDATES as u64 + 1;

/// Observation attributes a term may match — the same five the card's label is
/// built from, so everything searchable is also visible on the row that answers.
const FILTERABLE_VALUE_TYPES: [&str; 5] = [
    "email",
    "username",
    "display_name",
    "first_name",
    "last_name",
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
    visible_to: Option<VisibleTo<'_>>,
    after: Option<After<'_>>,
    limit: u64,
) -> anyhow::Result<Vec<PersonListRow>> {
    match narrowing(db, tenant_id, terms, within).await? {
        Narrowing::Nobody => Ok(Vec::new()),
        Narrowing::Tenant => page(db, tenant_id, terms, None, visible_to, after, limit).await,
        Narrowing::Persons(ids) => {
            page(db, tenant_id, terms, Some(&ids), visible_to, after, limit).await
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
    visible_to: Option<VisibleTo<'_>>,
    after: Option<After<'_>>,
    limit: u64,
) -> anyhow::Result<Vec<PersonListRow>> {
    let (sql, values) = build_query(tenant_id, terms, narrowed_to, visible_to, after, limit);
    let stmt = Statement::from_sql_and_values(DbBackend::MySql, &sql, values);
    rows_from(db.query_all(stmt).await?)
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
/// observations with no window in sight. One row past the cap is enough to know
/// the term is not selective, so that is where the probe stops.
async fn candidate_persons(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    first: &str,
    rest: &[String],
) -> anyhow::Result<Vec<Uuid>> {
    let (sql, values) = build_probe(tenant_id, first, rest);
    let stmt = Statement::from_sql_and_values(DbBackend::MySql, &sql, values);

    let rows = db.query_all(stmt).await?;
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
    let type_list = searched_types();
    let mut values: Vec<sea_orm::Value> = vec![tenant_id.as_bytes().to_vec().into()];

    values.push(like_pattern(first).into());

    // Every further term is the same clause with its own placeholder, so the
    // text repeats and only the binds differ — in the order the clauses appear.
    let clause = format!(
        "\n              AND EXISTS (SELECT 1 FROM persons t \
         WHERE t.insight_tenant_id = p.insight_tenant_id \
           AND t.person_id = p.person_id \
           AND t.value_type IN ({type_list}) \
           AND t.value_effective LIKE ? ESCAPE '!')"
    );
    let also = clause.repeat(rest.len());
    values.extend(rest.iter().map(|term| like_pattern(term).into()));

    values.push(PROBE_LIMIT.into());

    let sql = format!(
        r"
        SELECT DISTINCT p.person_id
        FROM persons p
        WHERE p.insight_tenant_id = ?
          AND p.value_type IN ({type_list})
          AND p.value_effective LIKE ? ESCAPE '!'{also}
        LIMIT ?
    "
    );

    (sql, values)
}

/// The listing with the probe deliberately skipped — the shape a term that names
/// more than [`MAX_CANDIDATES`] persons falls back to.
///
/// Test-only: the fallback must answer exactly what the narrowed path answers,
/// and only a live case over a real journal can say whether it does.
#[cfg(test)]
pub(super) async fn list_persons_unnarrowed(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    terms: &[String],
    after: Option<After<'_>>,
    limit: u64,
) -> anyhow::Result<Vec<PersonListRow>> {
    page(db, tenant_id, terms, None, None, after, limit).await
}

fn searched_types() -> String {
    FILTERABLE_VALUE_TYPES
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// How many persons the tenant has — the total behind the listing above.
///
/// INVARIANT: the set must stay identical to `build_query`'s `people` CTE. The
/// console prints this figure beside the list an operator pages through, so a
/// filter added to one side only makes the total disagree with what that list
/// can reach.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn count_persons(db: &DatabaseConnection, tenant_id: Uuid) -> anyhow::Result<usize> {
    const SQL: &str = "SELECT COUNT(DISTINCT person_id) AS persons FROM persons \
                       WHERE insight_tenant_id = ? AND person_id != ?";

    let stmt = Statement::from_sql_and_values(
        DbBackend::MySql,
        SQL,
        [
            tenant_id.as_bytes().to_vec().into(),
            EXCLUDED_PERSON.as_bytes().to_vec().into(),
        ],
    );

    let Some(row) = db.query_one(stmt).await? else {
        return Ok(0);
    };
    Ok(usize::try_from(row.try_get::<i64>("", "persons")?).unwrap_or(0))
}

/// The label half of the listing, the same for every query it serves: the value
/// current for each person × source × attribute, then the latest of those per
/// attribute, pivoted into the one label the row shows.
///
/// INVARIANT: this is `person_card`'s rule, and the module doc says why the two
/// steps cannot collapse into one.
const LABEL_CTES: &str = r"
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
        )";

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
    visible_to: Option<VisibleTo<'_>>,
    after: Option<After<'_>>,
    limit: u64,
) -> (String, Vec<sea_orm::Value>) {
    // INVARIANT: values are pushed in the order their placeholders appear in the
    // assembled statement — the roster, the ranking, the visible set, the
    // filters, the resume position, the limit. A fragment moved in the text moves
    // here too; a mismatch does not fail, it shifts every later binding by one.
    let mut values: Vec<sea_orm::Value> = Vec::new();
    let type_list = searched_types();

    values.push(tenant_id.as_bytes().to_vec().into());
    values.push(EXCLUDED_PERSON.as_bytes().to_vec().into());
    let roster = match narrowed_to {
        None => String::new(),
        Some(ids) => {
            let placeholders = vec!["?"; ids.len()].join(", ");
            values.extend(ids.iter().map(|id| id.as_bytes().to_vec().into()));
            format!("\n              AND person_id IN ({placeholders})")
        }
    };

    values.push(tenant_id.as_bytes().to_vec().into());
    // Ranking a narrowed roster is the whole point; reaching an unnarrowed one
    // through a semi-join is not — with nothing to narrow by, the plain scan the
    // browse case had all along stays.
    let ranked_within = if roster.is_empty() {
        ""
    } else {
        "\n              AND person_id IN (SELECT person_id FROM people)"
    };

    let scope = visible_scope(visible_to, tenant_id);
    let visible_cte = scope.cte;
    let recursive = scope.recursive;
    values.extend(scope.values);

    let mut filters = String::from(scope.filter);
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
        WITH {recursive}people AS (
            SELECT DISTINCT person_id
            FROM persons
            WHERE insight_tenant_id = ? AND person_id != ?{roster}
        ),
        ranked AS (
            SELECT person_id, value_type, value_effective, created_at, id,
                   ROW_NUMBER() OVER (
                       PARTITION BY person_id, insight_source_type, insight_source_id, value_type
                       ORDER BY created_at DESC, id DESC
                   ) AS rn
            FROM persons
            WHERE insight_tenant_id = ?
              AND value_type IN ({type_list}){ranked_within}
        ),
{LABEL_CTES}{visible_cte}
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

    fn query(terms: &[String], narrowed_to: Option<&[Uuid]>, after: Option<After<'_>>) -> String {
        build_query(Uuid::nil(), terms, narrowed_to, None, after, 50).0
    }

    fn visible_query(policy: VisibilityPolicy) -> String {
        build_query(
            Uuid::nil(),
            &[],
            None,
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
            sql.matches("FROM current_vals t").count(),
            terms.len(),
            "the exact filter must probe once per term: {sql}"
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
        assert!(
            sql.contains("AND person_id IN (SELECT person_id FROM people)"),
            "the ranking must read the cut roster: {sql}"
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
            sql.contains("FROM current_vals t"),
            "the exact filter still applies: {sql}"
        );
    }

    #[test]
    fn the_probe_requires_every_term_of_a_person() {
        let terms = ["iva".to_owned(), "example.com".to_owned()];

        let (sql, values) = build_probe(Uuid::nil(), &terms[0], &terms[1..]);

        assert_eq!(
            sql.matches("LIKE ?").count(),
            terms.len(),
            "one probe per term: {sql}"
        );
        assert!(!sql.contains("OR "), "terms are ANDed, never ORed: {sql}");
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
                // the roster, cut to the persons the caller narrowed to
                bytes(tenant),
                bytes(EXCLUDED_PERSON),
                bytes(within[0]),
                // the ranking over that roster
                bytes(tenant),
                // the exact filter, then the page position
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
                // the roster, cut to the persons the caller narrowed to
                bytes(tenant),
                bytes(EXCLUDED_PERSON),
                bytes(within[0]),
                // the ranking over that roster
                bytes(tenant),
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
