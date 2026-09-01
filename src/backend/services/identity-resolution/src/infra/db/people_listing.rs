use std::collections::BTreeMap;

use sea_orm::{ConnectionTrait as _, DatabaseConnection, DbBackend, QueryResult, Statement};
use uuid::Uuid;

use crate::config::VisibilityPolicy;

use super::visible_set_sql::CURRENT_VISIBLE_SET_CTE;

const ORDER_KEY_CHARS: usize = 255;

const _: () = assert!(ORDER_KEY_CHARS * 4 < 1024);

#[derive(Debug, thiserror::Error)]
pub(crate) enum PeopleListingError {
    #[error("people listing query failed")]
    Database(#[from] sea_orm::DbErr),
    #[error("people listing row decoding failed: {0}")]
    RowDecode(String),
    #[error("people listing row contains an invalid person id")]
    InvalidPersonId(#[from] uuid::Error),
    #[error("people listing row contains invalid attributes")]
    InvalidAttributes(#[from] serde_json::Error),
}

impl From<sea_orm::TryGetError> for PeopleListingError {
    fn from(error: sea_orm::TryGetError) -> Self {
        match error {
            sea_orm::TryGetError::DbErr(error) => Self::Database(error),
            sea_orm::TryGetError::Null(column) => Self::RowDecode(column),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Restrict<'a> {
    pub visible_to: Option<VisibleTo<'a>>,
}

impl Restrict<'_> {
    #[cfg(test)]
    const UNRESTRICTED: Self = Self { visible_to: None };
}

#[derive(Debug, Clone, Copy)]
pub struct VisibleTo<'a> {
    pub viewer_person_id: Uuid,
    pub org_source_type: &'a str,
    pub policy: VisibilityPolicy,
}

#[derive(Debug, Clone)]
pub struct PersonListRow {
    pub person_id: Uuid,
    pub email: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub manager_person_id: Option<Uuid>,
    pub order_key: String,
}

#[derive(Debug, Clone, Copy)]
pub struct After<'a> {
    pub order_key: &'a str,
    pub person_id: Uuid,
}

#[derive(Debug, Clone, Copy)]
pub struct ListQuery<'a> {
    pub org_chart_source_type: &'a str,
    pub terms: &'a [String],
    pub person_ids: &'a [Uuid],
    pub restrict: Restrict<'a>,
    pub after: Option<After<'a>>,
    pub limit: u64,
}

pub async fn list_persons(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    query: ListQuery<'_>,
) -> Result<Vec<PersonListRow>, PeopleListingError> {
    let (sql, values) = build_query(tenant_id, query);
    let statement = Statement::from_sql_and_values(DbBackend::MySql, &sql, values);

    rows_from(db.query_all_raw(statement).await?)
}

struct VisibleScopeSql {
    recursive: &'static str,
    cte: String,
    filter: &'static str,
    values: Vec<sea_orm::Value>,
}

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
        cte: format!(",\n{CURRENT_VISIBLE_SET_CTE}"),
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

fn build_query(tenant_id: Uuid, query: ListQuery<'_>) -> (String, Vec<sea_orm::Value>) {
    let mut values = vec![query.org_chart_source_type.to_owned().into()];
    values.push(tenant_id.as_bytes().to_vec().into());
    values.push(tenant_id.as_bytes().to_vec().into());
    let protect_manager = query.restrict.visible_to.is_some();
    let scope = visible_scope(query.restrict.visible_to, tenant_id);
    let recursive = scope.recursive;
    let visible_cte = scope.cte;
    let mut filters = String::from(scope.filter);
    let manager_projection = manager_projection(protect_manager);
    values.extend(scope.values);

    append_filters(&mut filters, &mut values, query.terms, query.person_ids);

    let resume = query.after.map_or_else(String::new, |_| {
        " WHERE (order_key > ? OR (order_key = ? AND person_id > ?))".to_owned()
    });
    if let Some(after) = query.after {
        values.push(after.order_key.into());
        values.push(after.order_key.into());
        values.push(after.person_id.as_bytes().to_vec().into());
    }
    values.push(query.limit.into());

    let sql = format!(
        r"
        WITH {recursive}ranked_managers AS (
            SELECT child_person_id,
                   parent_person_id,
                   ROW_NUMBER() OVER (
                       PARTITION BY child_person_id
                       ORDER BY insight_source_id
                   ) AS rn
            FROM org_chart
            WHERE insight_source_type = ?
              AND insight_tenant_id = ?
              AND valid_to IS NULL
              AND parent_person_id IS NOT NULL
        ),
        presented_people AS (
            SELECT p.insight_tenant_id,
                   p.person_id,
                   p.email,
                   p.username,
                   p.attributes,
                   oc.parent_person_id AS manager_person_id,
                   COALESCE(
                       NULLIF(TRIM(p.display_name), ''),
                       NULLIF(TRIM(CONCAT_WS(' ',
                           NULLIF(TRIM(p.first_name), ''),
                           NULLIF(TRIM(p.last_name), '')
                       )), '')
                   ) AS display_name,
                   p.first_name,
                   p.last_name,
                   COALESCE(
                       NULLIF(TRIM(p.display_name), ''),
                       NULLIF(TRIM(CONCAT_WS(' ',
                           NULLIF(TRIM(p.first_name), ''),
                           NULLIF(TRIM(p.last_name), '')
                       )), ''),
                       NULLIF(TRIM(p.username), ''),
                       NULLIF(TRIM(p.email), '')
                   ) AS label
            FROM people p
            LEFT JOIN ranked_managers oc
              ON  oc.child_person_id = p.person_id
              AND oc.rn = 1
            WHERE p.insight_tenant_id = ? AND p.valid_to IS NULL
        ){visible_cte}
        SELECT person_id, email, username, display_name, first_name, last_name,
               attributes, manager_person_id, order_key
        FROM (
            SELECT p.person_id,
                   p.email,
                   p.username,
                   p.display_name,
                   p.first_name,
                   p.last_name,
                   p.attributes,
                   {manager_projection} AS manager_person_id,
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

fn append_filters(
    filters: &mut String,
    values: &mut Vec<sea_orm::Value>,
    terms: &[String],
    person_ids: &[Uuid],
) {
    for person_id in person_ids {
        filters.push_str(" AND p.person_id = ?");
        values.push(person_id.as_bytes().to_vec().into());
    }

    for term in terms {
        filters.push_str(
            " AND (p.display_name LIKE ? ESCAPE '!' OR p.first_name LIKE ? ESCAPE '!' \
             OR p.last_name LIKE ? ESCAPE '!' OR p.username LIKE ? ESCAPE '!' \
             OR p.email LIKE ? ESCAPE '!')",
        );
        push_patterns(values, term);
    }
}

fn manager_projection(protect_manager: bool) -> &'static str {
    if protect_manager {
        return r"
                   CASE WHEN EXISTS (
                       SELECT 1
                       FROM visible_set vs
                       WHERE vs.person_id = p.manager_person_id
                   ) AND EXISTS (
                       SELECT 1
                       FROM people manager
                       WHERE manager.insight_tenant_id = p.insight_tenant_id
                         AND manager.person_id = p.manager_person_id
                         AND manager.valid_to IS NULL
                   ) THEN p.manager_person_id END";
    }
    r"
                   CASE WHEN EXISTS (
                       SELECT 1
                       FROM people manager
                       WHERE manager.insight_tenant_id = p.insight_tenant_id
                         AND manager.person_id = p.manager_person_id
                         AND manager.valid_to IS NULL
                   ) THEN p.manager_person_id END"
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

fn like_pattern(term: &str) -> String {
    let escaped = term
        .replace('!', "!!")
        .replace('%', "!%")
        .replace('_', "!_");
    format!("%{escaped}%")
}

fn rows_from(rows: Vec<QueryResult>) -> Result<Vec<PersonListRow>, PeopleListingError> {
    rows.into_iter()
        .map(|row| {
            Ok(PersonListRow {
                person_id: Uuid::from_slice(&row.try_get::<Vec<u8>>("", "person_id")?)?,
                email: row.try_get("", "email")?,
                username: row.try_get("", "username")?,
                display_name: row.try_get("", "display_name")?,
                first_name: row.try_get("", "first_name")?,
                last_name: row.try_get("", "last_name")?,
                attributes: serde_json::from_str(&row.try_get::<String>("", "attributes")?)?,
                manager_person_id: row
                    .try_get::<Option<Vec<u8>>>("", "manager_person_id")?
                    .map(|value| Uuid::from_slice(&value))
                    .transpose()?,
                order_key: row.try_get("", "order_key")?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(
        terms: &[String],
        person_ids: &[Uuid],
        restrict: Restrict<'_>,
        after: Option<After<'_>>,
    ) -> (String, Vec<sea_orm::Value>) {
        build_query(
            Uuid::from_u128(1),
            ListQuery {
                org_chart_source_type: "directory",
                terms,
                person_ids,
                restrict,
                after,
                limit: 50,
            },
        )
    }

    #[test]
    fn browsing_orders_current_people_by_presented_label() {
        let (sql, values) = query(&[], &[], Restrict::UNRESTRICTED, None);

        assert!(sql.contains("valid_to IS NULL"));
        assert!(sql.contains("SELECT p.insight_tenant_id,\n                   p.person_id"));
        assert!(sql.contains("manager_person_id"));
        assert!(sql.contains("ROW_NUMBER() OVER"));
        assert!(sql.contains("AND oc.rn = 1"));
        assert!(sql.contains("AND parent_person_id IS NOT NULL"));
        assert!(sql.contains("manager.valid_to IS NULL"));
        assert!(sql.contains("ORDER BY order_key, person_id"));
        assert!(sql.contains("NULLIF(TRIM(CONCAT_WS(' ',"));
        assert_eq!(sql.matches('?').count(), values.len());
    }

    #[test]
    fn every_term_and_named_person_narrows_the_page() {
        let person_id = Uuid::from_u128(2);
        let terms = ["50%".to_owned(), "name_with_underscore".to_owned()];
        let (sql, values) = query(&terms, &[person_id], Restrict::UNRESTRICTED, None);

        assert!(sql.contains("p.person_id = ?"));
        assert_eq!(sql.matches("p.display_name LIKE ?").count(), terms.len());
        assert_eq!(sql.matches('?').count(), values.len());
        assert!(values.contains(&sea_orm::Value::from("%50!%%".to_owned())));
        assert!(values.contains(&sea_orm::Value::from("%name!_with!_underscore%".to_owned())));
    }

    #[test]
    fn distinct_person_ids_are_separate_match_requirements() {
        let person_ids = [Uuid::from_u128(2), Uuid::from_u128(3)];
        let (sql, values) = query(&[], &person_ids, Restrict::UNRESTRICTED, None);

        assert_eq!(sql.matches("p.person_id = ?").count(), person_ids.len());
        assert_eq!(sql.matches('?').count(), values.len());
    }

    #[test]
    fn caller_visibility_and_resume_are_both_applied() {
        let after = After {
            order_key: "0example person",
            person_id: Uuid::from_u128(4),
        };
        let restrict = Restrict {
            visible_to: Some(VisibleTo {
                viewer_person_id: Uuid::from_u128(3),
                org_source_type: "directory",
                policy: VisibilityPolicy::OrgChart,
            }),
        };
        let (sql, values) = query(&[], &[], restrict, Some(after));

        assert!(sql.contains("WITH RECURSIVE"));
        assert!(sql.contains("FROM visible_set vs"));
        assert!(sql.contains("manager.valid_to IS NULL"));
        assert!(sql.contains("order_key > ?"));
        assert_eq!(sql.matches('?').count(), values.len());
    }
}
