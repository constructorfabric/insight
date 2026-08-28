//! One page, end to end: the position is checked against the question that
//! issued it, the one named input is read, and the page is reported with the
//! position the next one resumes from.
//!
//! INVARIANT: a page is served whole or refused whole — a position is never
//! issued for rows the caller was not given.

use sea_orm::DatabaseConnection;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::domain::compiler::drilldown::CompiledDrilldown;
use crate::domain::compiler::sql::CompiledMeasureQuery;

use super::super::catalog::MetricCatalog;
use super::super::dto::ServedFrom;
use super::super::error::QueryError;
use super::super::execute::fetch;
use super::super::provenance::{metric_versions, provenance};
use super::columns::PageShape;
use super::cursor::{Anchor, decode, encode, fingerprint, relation_snapshot, still_anchored};
use super::dto::RowsResponse;
use super::plan::plan;
use super::validation::ValidatedRows;

/// What one read answered.
type ReadRow = Map<String, Value>;

pub async fn answer(
    catalog: &MetricCatalog,
    clickhouse: &insight_clickhouse::Client,
    db: &DatabaseConnection,
    tenant_id: Uuid,
    request: ValidatedRows,
) -> Result<RowsResponse, QueryError> {
    let fingerprint = fingerprint(tenant_id, &request)?;
    let resume = request
        .cursor
        .as_deref()
        .map(|cursor| decode(cursor, &fingerprint))
        .transpose()?;

    let planned = plan(catalog, clickhouse, tenant_id, &request, resume.as_ref()).await?;
    let CompiledDrilldown {
        relation,
        contribution,
        columns,
        sql,
        params,
        ..
    } = planned.compiled;

    let comment = format!("metric-rows:{}:{}", request.input_role, request.metric_key);
    let statement = CompiledMeasureQuery { sql, params };
    let keys = [request.metric_key.clone()];
    let (read, versions) = tokio::join!(
        fetch::<ReadRow>(clickhouse, &statement, &comment),
        metric_versions(db, &keys)
    );

    let read = read?;

    // INVARIANT: the relation is identified after its rows were read, so a page
    // that spanned a rebuild is refused rather than served half from each.
    let anchor = Anchor {
        snapshot_id: relation_snapshot(clickhouse, &relation.database, &relation.relation).await?,
        identity_epoch: planned.identity_epoch,
    };
    if let Some(resume) = &resume {
        still_anchored(&resume.anchor, &anchor)?;
    }

    // INVARIANT: the shape is read off the statement rather than off the rows,
    // so an empty page reports the same columns a full one does.
    let shape = PageShape::of(&columns, contribution);
    let page = paged(read, request.page_size);
    let next_cursor = page
        .position
        .map(|last| encode(&fingerprint, &anchor, shape.position(&last)))
        .transpose()?;

    Ok(RowsResponse {
        provenance: provenance(&versions, &request.metric_key, ServedFrom::Computed),
        metric: request.metric_key,
        input: request.input_role,
        columns: shape.columns(),
        rows: page.rows.iter().map(|row| shape.row(row)).collect(),
        next_cursor,
    })
}

/// One page's rows, and the row a further page resumes after.
struct Page {
    rows: Vec<ReadRow>,
    /// Present exactly when a further page follows.
    position: Option<ReadRow>,
}

/// INVARIANT: the read binds one row beyond the page, so a further page is
/// detected without counting the whole result.
fn paged(mut read: Vec<ReadRow>, page_size: u32) -> Page {
    let size = page_size as usize;
    if read.len() <= size {
        return Page {
            rows: read,
            position: None,
        };
    }

    read.truncate(size);
    let position = read.last().cloned();
    Page {
        rows: read,
        position,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use chrono::NaiveDate;

    use super::super::super::catalog::product_metric_catalog;
    use super::super::super::fixtures::{SHIPPED_METRIC, offline_clickhouse, tenant};
    use super::*;

    fn row(marker: &str) -> ReadRow {
        let value = serde_json::json!({ "sort_0": marker });
        value.as_object().expect("an object").clone()
    }

    #[test]
    fn a_result_within_the_page_reports_no_position_to_resume_from() {
        let page = paged(vec![row("a"), row("b")], 2);

        assert_eq!(page.rows.len(), 2);
        assert!(page.position.is_none(), "nothing follows a full result");
    }

    #[test]
    fn one_row_beyond_the_page_is_dropped_and_marks_where_the_next_one_resumes() {
        let page = paged(vec![row("a"), row("b"), row("c")], 2);

        assert_eq!(page.rows.len(), 2);
        assert_eq!(
            page.position.as_ref().and_then(|last| last.get("sort_0")),
            Some(&Value::from("b")),
            "the next page resumes after the last row this one served"
        );
    }

    #[test]
    fn an_empty_result_is_a_page_of_no_rows_rather_than_a_further_page() {
        let page = paged(Vec::new(), 100);

        assert!(page.rows.is_empty());
        assert!(page.position.is_none());
    }

    #[tokio::test]
    async fn a_position_issued_for_another_question_is_refused_before_anything_is_read() {
        let asked = |to: u32| ValidatedRows {
            metric_key: SHIPPED_METRIC.to_owned(),
            subjects: vec![Uuid::from_u128(1)],
            from: NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date"),
            to: NaiveDate::from_ymd_opt(2026, 1, to).expect("valid date"),
            filters: Vec::new(),
            input_role: "value".to_owned(),
            display_dimensions: Vec::new(),
            page_size: 100,
            cursor: None,
        };
        let issued = fingerprint(tenant(), &asked(31)).expect("fingerprints");
        let cursor = encode(
            &issued,
            &Anchor {
                snapshot_id: "dataset-uuid".to_owned(),
                identity_epoch: 1,
            },
            vec!["row-7".to_owned()],
        )
        .expect("encodes");

        let error = answer(
            product_metric_catalog().expect("loads"),
            &offline_clickhouse(),
            &DatabaseConnection::default(),
            tenant(),
            ValidatedRows {
                cursor: Some(cursor),
                ..asked(30)
            },
        )
        .await
        .expect_err("a position of another question resumes nothing");

        assert!(matches!(error, QueryError::CursorMismatched), "{error}");
    }
}
