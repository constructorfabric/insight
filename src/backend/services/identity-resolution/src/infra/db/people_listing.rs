use sea_orm::{ConnectionTrait as _, DatabaseConnection, DbBackend, QueryResult, Statement};
use uuid::Uuid;

use crate::config::VisibilityPolicy;

const ORDER_KEY_CHARS: usize = 255;

const _: () = assert!(ORDER_KEY_CHARS * 4 < 1024);

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
    pub order_key: String,
}

#[derive(Debug, Clone, Copy)]
pub struct After<'a> {
    pub order_key: &'a str,
    pub person_id: Uuid,
}

pub async fn list_persons(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    terms: &[String],
    person_ids: &[Uuid],
    restrict: Restrict<'_>,
    after: Option<After<'_>>,
    limit: u64,
) -> anyhow::Result<Vec<PersonListRow>> {
    let (sql, values) = build_query(tenant_id, terms, person_ids, restrict, after, limit);
    let statement = Statement::from_sql_and_values(DbBackend::MySql, &sql, values);

    rows_from(db.query_all_raw(statement).await?)
}

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
    person_ids: &[Uuid],
    restrict: Restrict<'_>,
    after: Option<After<'_>>,
    limit: u64,
) -> (String, Vec<sea_orm::Value>) {
    let mut values = vec![tenant_id.as_bytes().to_vec().into()];
    let scope = visible_scope(restrict.visible_to, tenant_id);
    let recursive = scope.recursive;
    let visible_cte = scope.cte;
    let mut filters = String::from(scope.filter);
    values.extend(scope.values);

    if !person_ids.is_empty() {
        let placeholders = vec!["?"; person_ids.len()].join(", ");
        filters.push_str(" AND p.person_id IN (");
        filters.push_str(&placeholders);
        filters.push(')');
        values.extend(
            person_ids
                .iter()
                .map(|person_id| person_id.as_bytes().to_vec().into()),
        );
    }

    for term in terms {
        filters.push_str(
            " AND (p.display_name LIKE ? ESCAPE '!' OR p.first_name LIKE ? ESCAPE '!' \
             OR p.last_name LIKE ? ESCAPE '!' OR p.username LIKE ? ESCAPE '!' \
             OR p.email LIKE ? ESCAPE '!')",
        );
        push_patterns(&mut values, term);
    }

    let resume = after.map_or_else(String::new, |_| {
        " WHERE (order_key > ? OR (order_key = ? AND person_id > ?))".to_owned()
    });
    if let Some(after) = after {
        values.push(after.order_key.into());
        values.push(after.order_key.into());
        values.push(after.person_id.as_bytes().to_vec().into());
    }
    values.push(limit.into());

    let sql = format!(
        r"
        WITH {recursive}presented_people AS (
            SELECT person_id,
                   email,
                   username,
                   COALESCE(
                       NULLIF(TRIM(display_name), ''),
                       NULLIF(TRIM(CONCAT_WS(' ',
                           NULLIF(TRIM(first_name), ''),
                           NULLIF(TRIM(last_name), '')
                       )), '')
                   ) AS display_name,
                   first_name,
                   last_name,
                   COALESCE(
                       NULLIF(TRIM(display_name), ''),
                       NULLIF(TRIM(CONCAT_WS(' ',
                           NULLIF(TRIM(first_name), ''),
                           NULLIF(TRIM(last_name), '')
                       )), ''),
                       NULLIF(TRIM(username), ''),
                       NULLIF(TRIM(email), '')
                   ) AS label
            FROM people
            WHERE insight_tenant_id = ? AND valid_to IS NULL
        ){visible_cte}
        SELECT person_id, email, username, display_name, first_name, last_name, order_key
        FROM (
            SELECT p.person_id,
                   p.email,
                   p.username,
                   p.display_name,
                   p.first_name,
                   p.last_name,
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

fn like_pattern(term: &str) -> String {
    let escaped = term
        .replace('!', "!!")
        .replace('%', "!%")
        .replace('_', "!_");
    format!("%{escaped}%")
}

fn rows_from(rows: Vec<QueryResult>) -> anyhow::Result<Vec<PersonListRow>> {
    rows.into_iter()
        .map(|row| {
            Ok(PersonListRow {
                person_id: Uuid::from_slice(&row.try_get::<Vec<u8>>("", "person_id")?)?,
                email: row.try_get("", "email")?,
                username: row.try_get("", "username")?,
                display_name: row.try_get("", "display_name")?,
                first_name: row.try_get("", "first_name")?,
                last_name: row.try_get("", "last_name")?,
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
        build_query(Uuid::from_u128(1), terms, person_ids, restrict, after, 50)
    }

    #[test]
    fn browsing_orders_current_people_by_presented_label() {
        let (sql, values) = query(&[], &[], Restrict::UNRESTRICTED, None);

        assert!(sql.contains("valid_to IS NULL"));
        assert!(sql.contains("ORDER BY order_key, person_id"));
        assert!(sql.contains("NULLIF(TRIM(CONCAT_WS(' ',"));
        assert_eq!(sql.matches('?').count(), values.len());
    }

    #[test]
    fn every_term_and_named_person_narrows_the_page() {
        let person_id = Uuid::from_u128(2);
        let terms = ["50%".to_owned(), "name_with_underscore".to_owned()];
        let (sql, values) = query(&terms, &[person_id], Restrict::UNRESTRICTED, None);

        assert!(sql.contains("p.person_id IN (?)"));
        assert_eq!(sql.matches("p.display_name LIKE ?").count(), terms.len());
        assert_eq!(sql.matches('?').count(), values.len());
        assert!(values.contains(&sea_orm::Value::from("%50!%%".to_owned())));
        assert!(values.contains(&sea_orm::Value::from("%name!_with!_underscore%".to_owned())));
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
        assert!(sql.contains("order_key > ?"));
        assert_eq!(sql.matches('?').count(), values.len());
    }
}
