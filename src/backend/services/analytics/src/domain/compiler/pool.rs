//! How a read reaches its entities when the caller named people rather than
//! identities: one bound (person, identity) pair per resolution, joined on the
//! measure's entity field, so a person's identities fold into one row group and
//! a person no row matches reports nothing rather than a zero.

use super::error::CompileError;
use super::request::{EntityScope, ResolvedPerson};
use super::sql::QueryParam;

/// The person column beside the rows the join produced.
const JOINED_PERSON: &str = "pool.person_ref";
/// The same column read from a stage the join already passed through.
const CARRIED_PERSON: &str = "person_ref";

/// The caller's identity resolution as a relation the statement binds.
pub(super) struct Pool<'a> {
    members: &'a [ResolvedPerson],
    /// What an empty pool is called in the error it raises.
    selection: &'static str,
}

impl<'a> Pool<'a> {
    /// The pool a scope reads through; a scope narrowing by predicate has none.
    pub fn of_scope(scope: &'a EntityScope) -> Option<Self> {
        match scope {
            EntityScope::Tenant | EntityScope::Identities(_) => None,
            EntityScope::People(members) => Some(Self {
                members,
                selection: "the entity scope",
            }),
        }
    }

    /// The pool a comparison read compares its targets against.
    pub fn of_peers(members: &'a [ResolvedPerson]) -> Self {
        Self {
            members,
            selection: "the comparison pool",
        }
    }

    /// The `pool` relation as one common-table expression, one bound pair per
    /// resolved (person, identity).
    // INVARIANT: every pooled read writes this before anything else it binds,
    // so the pairs are always the statement's first parameters.
    pub fn cte(&self, params: &mut Vec<QueryParam>) -> Result<String, CompileError> {
        let pairs: usize = self
            .members
            .iter()
            .map(|member| member.identities.len())
            .sum();
        if pairs == 0 {
            return Err(CompileError::EmptySelection {
                selection: self.selection.to_owned(),
            });
        }

        for member in self.members {
            for identity in &member.identities {
                params.push(QueryParam::Text(member.person_ref.clone()));
                params.push(QueryParam::Text(identity.clone()));
            }
        }

        let tuples = vec!["(?, ?)"; pairs].join(", ");
        Ok(format!(
            "pool AS (\n    SELECT\n        member.1 AS person_ref,\n        member.2 AS identity\n    FROM (SELECT arrayJoin([{tuples}]) AS member)\n)"
        ))
    }
}

/// The head of a read whose pool is its only common-table expression.
pub(super) fn only_cte(
    pool: Option<&Pool<'_>>,
    params: &mut Vec<QueryParam>,
) -> Result<String, CompileError> {
    match pool {
        None => Ok(String::new()),
        Some(pool) => Ok(format!("WITH {}\n", pool.cte(params)?)),
    }
}

/// The head of a read that opens further common-table expressions after the
/// pool, which the caller writes next.
pub(super) fn first_cte(
    pool: Option<&Pool<'_>>,
    params: &mut Vec<QueryParam>,
) -> Result<String, CompileError> {
    match pool {
        None => Ok("WITH ".to_owned()),
        Some(pool) => Ok(format!("WITH {},\n", pool.cte(params)?)),
    }
}

/// The pool as one more common-table expression in a head another stage opened.
pub(super) fn continued_cte(
    pool: &Pool<'_>,
    params: &mut Vec<QueryParam>,
) -> Result<String, CompileError> {
    Ok(format!("{},\n", pool.cte(params)?))
}

/// The relation a read scans, with the join that narrows it to the pool.
/// `indent` is the leading whitespace the statement's own clause lines carry.
pub(super) fn scan_clause(
    relation: String,
    pool: Option<&Pool<'_>>,
    entity: &str,
    indent: &str,
) -> String {
    match pool {
        None => relation,
        Some(_) => {
            format!("{relation}\n{indent}INNER JOIN pool ON pool.identity = {entity}")
        }
    }
}

/// What a read keys its rows by in the stage that carries the join.
pub(super) fn joined_entity<'a>(pool: Option<&Pool<'_>>, entity: &'a str) -> &'a str {
    match pool {
        None => entity,
        Some(_) => JOINED_PERSON,
    }
}

/// The same, read from a stage the join has already passed through.
pub(super) fn carried_entity<'a>(pool: Option<&Pool<'_>>, entity: &'a str) -> &'a str {
    match pool {
        None => entity,
        Some(_) => CARRIED_PERSON,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::domain::compiler::error::CompileError;
    use crate::domain::compiler::fixtures::{
        bins_view, compile, compile_err, direct, labelled_measure, lines, measure, metric, people,
        people_params, percentile, pool_head, query, sized_measure, text,
    };
    use crate::domain::compiler::request::{
        CombinedSplitView, EntityScope, GroupLimit, RankedDimension, RankedGroup, ResolvedPerson,
        SubjectSeriesView, SubjectSplitView, ViewKind,
    };
    use crate::domain::compiler::sql::{CompiledMeasureQuery, QueryParam};
    use crate::domain::definitions::definition::{Aggregation, MeasureDefinition};

    fn cap() -> GroupLimit {
        GroupLimit {
            groups: vec![RankedGroup {
                rank: 1,
                dimensions: vec![RankedDimension {
                    value: "example/app".to_owned(),
                    label: None,
                }],
            }],
            include_remainder: true,
        }
    }

    fn subject_series(group_limit: Option<GroupLimit>) -> ViewKind {
        let dimensions = if group_limit.is_some() {
            vec!["repository".to_owned()]
        } else {
            Vec::new()
        };
        ViewKind::SubjectSeries(SubjectSeriesView {
            dimensions,
            group_limit,
        })
    }

    fn every_view() -> Vec<ViewKind> {
        vec![
            ViewKind::SubjectTotal,
            subject_series(None),
            subject_series(Some(cap())),
            ViewKind::SubjectSplit(SubjectSplitView {
                dimensions: vec!["repository".to_owned()],
            }),
            ViewKind::CombinedSplit(CombinedSplitView {
                dimensions: vec!["repository".to_owned()],
                group_limit: None,
            }),
            ViewKind::CombinedSplit(CombinedSplitView {
                dimensions: vec!["repository".to_owned()],
                group_limit: Some(cap()),
            }),
        ]
    }

    fn over_people(view: ViewKind) -> CompiledMeasureQuery {
        let mut request = query(view);
        request.entity_scope = people();
        compile(
            &metric(direct("prs_merged")),
            &[labelled_measure("prs_merged")],
            &request,
        )
    }

    #[test]
    fn a_people_scoped_read_keys_each_row_by_the_person_its_pool_attributes_it_to() {
        let compiled = over_people(ViewKind::SubjectTotal);

        assert_eq!(
            compiled.sql,
            lines(&[
                "WITH pool AS (",
                "    SELECT",
                "        member.1 AS person_ref,",
                "        member.2 AS identity",
                "    FROM (SELECT arrayJoin([(?, ?), (?, ?), (?, ?)]) AS member)",
                ")",
                "SELECT",
                "    pool.person_ref AS entity_id,",
                "    toFloat64(count()) AS value",
                "FROM silver.class_git_pull_requests FINAL",
                "INNER JOIN pool ON pool.identity = author_email",
                "WHERE tenant_id = ?",
                "  AND toDate(closed_on) >= toDate(?)",
                "  AND toDate(closed_on) <= toDate(?)",
                "GROUP BY entity_id",
                "LIMIT ?",
            ])
        );
        assert_eq!(
            compiled.params,
            [
                people_params(),
                vec![
                    text("acme-tenant"),
                    text("2026-01-01"),
                    text("2026-01-31"),
                    QueryParam::UInt(10_001),
                ]
            ]
            .concat()
        );
    }

    #[test]
    fn every_view_over_a_people_scope_opens_with_the_pool_and_binds_its_pairs_first() {
        for view in every_view() {
            let name = view.name();
            let compiled = over_people(view);

            assert!(
                compiled.sql.starts_with(&lines(&pool_head())),
                "{name}: {}",
                compiled.sql
            );
            assert!(
                compiled
                    .sql
                    .contains("INNER JOIN pool ON pool.identity = author_email"),
                "{name}: {}",
                compiled.sql
            );
            assert!(
                !compiled.sql.contains("author_email IN ("),
                "the pool join narrows the read, not a predicate: {name}"
            );
            assert_eq!(
                compiled.params.get(..people_params().len()),
                Some(people_params().as_slice()),
                "{name}"
            );
            assert_eq!(
                compiled.sql.matches('?').count(),
                compiled.params.len(),
                "{name}"
            );
        }
    }

    #[test]
    fn a_capped_read_carries_the_person_through_its_ranked_scan() {
        let capped = over_people(subject_series(Some(cap())));
        let combined_split = over_people(ViewKind::CombinedSplit(CombinedSplitView {
            dimensions: vec!["repository".to_owned()],
            group_limit: Some(cap()),
        }));

        assert!(capped.sql.contains("        person_ref AS entity_id,"));
        assert!(
            combined_split
                .sql
                .contains("        uniqExact(person_ref) AS contributing_entity_count")
        );
    }

    #[test]
    fn a_combined_split_over_people_counts_the_people_that_contributed_not_their_identities() {
        let compiled = over_people(ViewKind::CombinedSplit(CombinedSplitView {
            dimensions: vec!["repository".to_owned()],
            group_limit: None,
        }));

        assert!(
            compiled
                .sql
                .contains("    uniqExact(pool.person_ref) AS contributing_entity_count,"),
            "{}",
            compiled.sql
        );
    }

    #[test]
    fn a_bins_read_over_people_bins_each_persons_own_observations() {
        let mut request = query(bins_view(10));
        request.entity_scope = people();

        let compiled = compile(
            &metric(percentile("pr_size", 0.5)),
            &[sized_measure("pr_size")],
            &request,
        );

        assert!(compiled.sql.starts_with(&lines(&[
            "WITH pool AS (",
            "    SELECT",
            "        member.1 AS person_ref,",
            "        member.2 AS identity",
            "    FROM (SELECT arrayJoin([(?, ?), (?, ?), (?, ?)]) AS member)",
            "),",
            "raw_events AS (",
            "    SELECT",
            "        pool.person_ref AS entity_id,",
        ])));
        assert!(
            compiled
                .sql
                .contains("    INNER JOIN pool ON pool.identity = author_email")
        );
    }

    #[test]
    fn counting_distinct_days_over_a_persons_two_identities_counts_a_shared_day_once() {
        let active_days = MeasureDefinition {
            aggregation: Aggregation::CountDistinct,
            subject_expr: Some("toDate(closed_on)".to_owned()),
            ..measure("active_days", None)
        };
        let mut request = query(ViewKind::SubjectTotal);
        request.entity_scope = people();

        let compiled = compile(&metric(direct("active_days")), &[active_days], &request);

        assert!(compiled.sql.contains(&lines(&[
            "    pool.person_ref AS entity_id,",
            "    toFloat64(uniqExact(toDate(closed_on))) AS value",
        ])));
        assert!(compiled.sql.contains("GROUP BY entity_id"));
    }

    /// The compiler is the backstop behind the plan, which answers a question
    /// nobody's identities reach without compiling anything at all.
    #[test]
    fn a_people_scope_no_identity_resolved_for_is_refused_rather_than_read_tenant_wide() {
        let mut request = query(ViewKind::SubjectTotal);
        request.entity_scope = EntityScope::People(vec![ResolvedPerson {
            person_ref: "person-1".to_owned(),
            identities: Vec::new(),
        }]);

        assert_eq!(
            compile_err(
                &metric(direct("prs_merged")),
                &[labelled_measure("prs_merged")],
                &request
            ),
            CompileError::EmptySelection {
                selection: "the entity scope".to_owned(),
            }
        );
    }

    #[test]
    fn an_identity_scope_still_narrows_by_predicate_and_opens_no_pool() {
        let mut request = query(ViewKind::SubjectTotal);
        request.entity_scope = EntityScope::Identities(vec!["one@example.com".to_owned()]);

        let compiled = compile(
            &metric(direct("prs_merged")),
            &[labelled_measure("prs_merged")],
            &request,
        );

        assert!(
            compiled
                .sql
                .starts_with("SELECT\n    author_email AS entity_id,")
        );
        assert!(!compiled.sql.contains("pool"));
        assert!(compiled.sql.contains("  AND author_email IN (?)"));
    }
}
