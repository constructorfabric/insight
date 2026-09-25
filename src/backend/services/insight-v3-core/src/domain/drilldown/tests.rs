use serde_json::json;

use super::cursor::{CursorKey, KeyValue, decode, encode, fingerprint};
use super::order::{OrderKey, is_reserved, presented};
use super::paged::wrap;
use super::*;
use crate::domain::query::metric_query::{
    ColumnKind, FilterBind, MetricQuery, People, RECORD_COLUMN, TWIN_COLUMN, TableEngine,
};
use crate::domain::query::time_window::Window;

fn column(key: &str, kind: ColumnKind) -> Column {
    Column {
        key: key.to_owned(),
        kind,
        percent: false,
    }
}

fn columns() -> Vec<Column> {
    vec![
        column("day", ColumnKind::Text),
        column("total", ColumnKind::Number),
        column("when", ColumnKind::Date),
    ]
}

fn sort(key: &str, descending: bool) -> Sort {
    Sort {
        key: key.to_owned(),
        descending,
    }
}

/// A key over `columns()` ordered by `day`: the flag, the day, then every
/// column as text.
fn a_key() -> CursorKey {
    CursorKey {
        flag: false,
        value: KeyValue::Text("2026-09-01".to_owned()),
        ties: vec![
            "2026-09-01".to_owned(),
            "2".to_owned(),
            "2026-09-01 00:00:00".to_owned(),
        ],
    }
}

#[test]
fn the_key_leads_with_the_blank_flag_and_ends_with_every_column_as_text() {
    let order = OrderKey::build(&columns(), &sort("day", false), &[]);

    let projection = order.projection();
    assert!(
        projection.contains(
            "toUInt8((isNull(__m.`day`) OR toString(__m.`day`) = '')) AS `__drilldown_flag`"
        ),
        "{projection}"
    );
    assert!(
        projection.contains("ifNull(toString(__m.`day`), '') AS `__drilldown_key`"),
        "{projection}"
    );
    for (index, name) in ["day", "total", "when"].iter().enumerate() {
        assert!(
            projection.contains(&format!(
                "ifNull(toString(__m.`{name}`), '') AS `__drilldown_tie{index}`"
            )),
            "{projection}"
        );
    }
    assert_eq!(
        order.order_by(),
        "`__drilldown_flag` ASC, `__drilldown_key` ASC, `__drilldown_tie0` ASC, \
         `__drilldown_tie1` ASC, `__drilldown_tie2` ASC"
    );
}

/// A blank cell sits past every filled one whichever way the column runs:
/// the flag that leads the key is inverted under a downward order. For a
/// number, a cell that is not a finite number is blank too.
#[test]
fn a_number_is_compared_as_a_number_and_a_downward_order_runs_every_element_down() {
    let order = OrderKey::build(&columns(), &sort("total", true), &[]);

    let projection = order.projection();
    assert!(
        projection.contains(
            "toUInt8(NOT (isNull(toFloat64OrNull(toString(__m.`total`))) OR NOT \
             isFinite(toFloat64OrNull(toString(__m.`total`))))) AS `__drilldown_flag`"
        ),
        "{projection}"
    );
    assert!(
        projection.contains(
            "ifNotFinite(ifNull(toFloat64OrNull(toString(__m.`total`)), 0), 0) AS `__drilldown_key`"
        ),
        "{projection}"
    );
    assert_eq!(
        order.order_by(),
        "`__drilldown_flag` DESC, `__drilldown_key` DESC, `__drilldown_tie0` DESC, \
         `__drilldown_tie1` DESC, `__drilldown_tie2` DESC"
    );
}

#[test]
fn a_hidden_column_the_query_carries_is_the_last_element_of_the_key() {
    let order = OrderKey::build(&columns(), &sort("day", false), &[RECORD_COLUMN.to_owned()]);

    assert!(
        order.projection().contains(&format!(
            "ifNull(toString(__m.`{RECORD_COLUMN}`), '') AS `__drilldown_tie3`"
        )),
        "{}",
        order.projection()
    );
}

#[test]
fn the_cursor_predicate_binds_the_flag_the_key_and_every_tie_in_tuple_order() {
    let order = OrderKey::build(&columns(), &sort("day", false), &[]);
    let mut binds = Vec::new();

    let predicate = order.cursor_predicate(&a_key(), &mut binds);

    assert!(
        predicate.starts_with("tuple(toUInt8((isNull(__m.`day`)"),
        "{predicate}"
    );
    assert!(
        predicate.ends_with(") > tuple(?, ?, ?, ?, ?)"),
        "{predicate}"
    );
    assert!(matches!(
        binds.as_slice(),
        [
            FilterBind::Bool(false),
            FilterBind::Str(_),
            FilterBind::Str(_),
            FilterBind::Str(_),
            FilterBind::Str(_)
        ]
    ));
}

#[test]
fn a_downward_order_resumes_past_the_cursor_with_the_opposite_comparison() {
    let order = OrderKey::build(&columns(), &sort("total", true), &[]);
    let key = CursorKey {
        flag: false,
        value: KeyValue::Number(2.0),
        ties: vec![String::new(), String::new(), String::new()],
    };
    let mut binds = Vec::new();

    let predicate = order.cursor_predicate(&key, &mut binds);

    assert!(predicate.contains(") < tuple("), "{predicate}");
    assert!(
        matches!(binds.as_slice(), [FilterBind::Bool(false), FilterBind::Float(value), ..] if (*value - 2.0).abs() < f64::EPSILON)
    );
}

/// A key the order did not issue would reach the warehouse as a comparison
/// it refuses, and that refusal would read as ours. It is the caller's.
#[test]
fn a_key_that_does_not_fit_the_order_is_refused_before_it_is_bound() {
    let by_text = OrderKey::build(&columns(), &sort("day", false), &[]);
    let by_number = OrderKey::build(&columns(), &sort("total", false), &[]);
    let ties = |count: usize| vec![String::new(); count];
    let cases = [
        (
            "a number where the order compares text",
            &by_text,
            CursorKey {
                flag: false,
                value: KeyValue::Number(1.0),
                ties: ties(3),
            },
        ),
        (
            "text where the order compares numbers",
            &by_number,
            CursorKey {
                flag: false,
                value: KeyValue::Text("1".to_owned()),
                ties: ties(3),
            },
        ),
        (
            "a number that is not finite",
            &by_number,
            CursorKey {
                flag: false,
                value: KeyValue::Number(f64::NAN),
                ties: ties(3),
            },
        ),
        (
            "too short a tail",
            &by_text,
            CursorKey {
                flag: false,
                value: KeyValue::Text("x".to_owned()),
                ties: ties(2),
            },
        ),
        (
            "too long a tail",
            &by_text,
            CursorKey {
                flag: false,
                value: KeyValue::Text("x".to_owned()),
                ties: ties(4),
            },
        ),
    ];

    for (label, order, key) in cases {
        assert!(
            matches!(order.check(&key), Err(CursorError::Malformed)),
            "should refuse: {label}"
        );
    }
    assert!(by_text.check(&a_key()).is_ok());
}

#[test]
fn the_key_of_a_row_is_read_off_the_cells_the_query_projected() {
    let order = OrderKey::build(&columns(), &sort("total", false), &[]);
    let row = json!({
        "day": "2026-09-01", "total": "2", "when": "2026-09-01 00:00:00",
        "__drilldown_flag": 0, "__drilldown_key": 2.0,
        "__drilldown_tie0": "2026-09-01", "__drilldown_tie1": "2", "__drilldown_tie2": "2026-09-01 00:00:00"
    });
    let row = row.as_object().cloned().unwrap_or_default();

    let key = order
        .key_of(&row)
        .unwrap_or_else(|error| panic!("the key reads: {error}"));

    assert!(!key.flag);
    assert!(matches!(key.value, KeyValue::Number(value) if (value - 2.0).abs() < f64::EPSILON));
    assert_eq!(key.ties, vec!["2026-09-01", "2", "2026-09-01 00:00:00"]);
}

#[test]
fn a_row_is_presented_without_the_working_cells_and_with_its_numbers_as_numbers() {
    let row = json!({
        "day": "2026-09-01", "total": "2", "when": "2026-09-01 00:00:00",
        "__drilldown_flag": 0, "__drilldown_key": "2026-09-01",
        "__drilldown_tie0": "x", "__drilldown_tie1": "2", "__drilldown_tie2": "x",
        "__drilldown_record": "0193"
    });
    let row = row.as_object().cloned().unwrap_or_default();

    let shown = presented(row, &columns(), &[RECORD_COLUMN.to_owned()]);

    assert_eq!(
        serde_json::Value::Object(shown),
        json!({ "day": "2026-09-01", "total": 2, "when": "2026-09-01 00:00:00" })
    );
}

#[test]
fn the_names_the_wrapper_writes_are_kept_from_a_metrics_columns() {
    for (name, reserved) in [
        ("__drilldown_flag", true),
        ("__drilldown_key", true),
        ("__drilldown_tie7", true),
        (RECORD_COLUMN, true),
        (TWIN_COLUMN, true),
        ("day", false),
        ("__mine", false),
    ] {
        assert_eq!(is_reserved(name), reserved, "{name}");
    }
}

#[test]
fn a_cursor_survives_the_round_trip_and_refuses_what_it_did_not_issue() {
    let written = encode("fp", "snap", &Window::legacy(), a_key());

    let envelope = decode(&written).unwrap_or_else(|error| panic!("decodes: {error}"));
    assert_eq!(envelope.fingerprint, "fp");
    assert_eq!(envelope.snapshot, "snap");
    assert_eq!(envelope.window, Window::legacy());
    assert_eq!(envelope.key.ties, a_key().ties);

    assert!(matches!(decode("not base64!"), Err(CursorError::Malformed)));

    let other_version = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        json!({ "version": 99, "fingerprint": "fp", "snapshot": "", "window": "Unwindowed", "key": { "flag": false, "value": "x", "ties": [] } }).to_string(),
    );
    assert!(matches!(decode(&other_version), Err(CursorError::Version)));
}

/// Everything one page is bound to, so a case can vary one input.
struct Bound {
    name: &'static str,
    body: serde_json::Value,
    range: &'static str,
    bucket: Option<bool>,
    window: Window,
    sort: Sort,
    columns: Vec<Column>,
}

impl Bound {
    fn digest(&self) -> String {
        fingerprint(
            self.name,
            &self.body,
            Some(self.range),
            self.bucket,
            &self.window,
            &self.sort,
            &self.columns,
        )
    }
}

fn a_bound_page() -> Bound {
    Bound {
        name: "m",
        body: json!({ "table": "events", "fields": [] }),
        range: "P30D",
        bucket: None,
        window: Window::legacy(),
        sort: sort("day", false),
        columns: columns(),
    }
}

#[test]
fn the_fingerprint_moves_with_anything_a_page_is_bound_to() {
    let base = a_bound_page().digest();
    assert_eq!(base, a_bound_page().digest());

    let unbounded = Window::Requested {
        bounds: crate::domain::query::time_window::Bounds::Unbounded,
        grain: None,
    };
    let cases = [
        (
            "the metric's name",
            Bound {
                name: "n",
                ..a_bound_page()
            },
        ),
        (
            "the metric's body",
            Bound {
                body: json!({ "table": "other", "fields": [] }),
                ..a_bound_page()
            },
        ),
        (
            "the range as written",
            Bound {
                range: "P7D",
                ..a_bound_page()
            },
        ),
        (
            "the bucketing",
            Bound {
                bucket: Some(false),
                ..a_bound_page()
            },
        ),
        (
            "the window as resolved",
            Bound {
                window: unbounded,
                ..a_bound_page()
            },
        ),
        (
            "the order",
            Bound {
                sort: sort("day", true),
                ..a_bound_page()
            },
        ),
        (
            "the columns",
            Bound {
                columns: columns().drain(..1).collect(),
                ..a_bound_page()
            },
        ),
    ];
    for (label, other) in cases {
        assert_ne!(base, other.digest(), "should differ: {label}");
    }
}

#[test]
fn a_page_asked_for_in_no_order_takes_the_metrics_own_or_the_first_column() {
    let own = ("total".to_owned(), true);

    assert_eq!(default_sort(Some(&own), &columns()), sort("total", true));
    assert_eq!(default_sort(None, &columns()), sort("day", false));

    let gone = ("elsewhere".to_owned(), true);
    assert_eq!(default_sort(Some(&gone), &columns()), sort("day", false));
}

fn compiled(value: serde_json::Value) -> crate::domain::query::metric_query::CompiledQuery {
    let metric: MetricQuery =
        serde_json::from_value(value).unwrap_or_else(|error| panic!("valid metric: {error}"));

    metric
        .compile_paged(
            &People::new("identity"),
            &Window::legacy(),
            TableEngine::Other,
            None,
        )
        .unwrap_or_else(|error| panic!("compiles: {error}"))
}

fn grouped_over_a_table() -> crate::domain::query::metric_query::CompiledQuery {
    compiled(json!({
        "table": "events",
        "fields": [
            { "column": "day", "type": "string", "as_name": "day" },
            { "column": "lines", "type": "int", "agg": "sum", "as_name": "total" }
        ],
        "group_by": ["day"],
        "filters": [{ "column": "repo", "type": "string", "op": "eq", "value": "app" }]
    }))
}

#[test]
fn the_wrapper_reads_the_metric_as_a_subquery_and_asks_one_row_past_the_page() {
    let inner = grouped_over_a_table();
    let order = OrderKey::build(&columns()[..2], &sort("day", false), inner.hidden());

    let (sql, binds) = wrap(&inner, &order, None, 2, &columns()[..2]);

    assert!(sql.starts_with("SELECT __m.*, "), "{sql}");
    assert!(sql.contains(" FROM (SELECT "), "{sql}");
    assert!(
        sql.contains(
            ") AS __m ORDER BY `__drilldown_flag` ASC, `__drilldown_key` ASC, \
             `__drilldown_tie0` ASC, `__drilldown_tie1` ASC LIMIT 3"
        ),
        "{sql}"
    );
    assert!(!sql.contains(" WHERE tuple("), "{sql}");
    assert!(matches!(binds.as_slice(), [FilterBind::Str(repo)] if repo == "app"));
}

/// The inner query binds first because it is written first: a cursor bound
/// ahead of a filter would take the filter's value and shift the rest.
#[test]
fn a_resumed_page_binds_the_metrics_own_values_before_the_cursors() {
    let inner = grouped_over_a_table();
    let order = OrderKey::build(&columns()[..2], &sort("day", false), inner.hidden());
    let key = CursorKey {
        flag: false,
        value: KeyValue::Text("2026-09-01".to_owned()),
        ties: vec!["2026-09-01".to_owned(), "2".to_owned()],
    };

    let (sql, binds) = wrap(&inner, &order, Some(&key), 2, &columns()[..2]);

    assert!(sql.contains(") AS __m WHERE tuple("), "{sql}");
    assert!(matches!(
        binds.as_slice(),
        [FilterBind::Str(repo), FilterBind::Bool(false), FilterBind::Str(day), FilterBind::Str(tie), FilterBind::Str(total)]
            if repo == "app" && day == "2026-09-01" && tie == "2026-09-01" && total == "2"
    ));
}

/// Rows a warehouse table cannot tell apart are counted off before the
/// page is cut, so a hundred and fifty of one kind come back as a hundred
/// and then fifty, not as a hundred and then nothing.
#[test]
fn a_plain_read_over_a_table_numbers_its_twins_in_a_layer_of_its_own() {
    let inner = compiled(json!({
        "table": "events",
        "fields": [
            { "column": "day", "type": "string", "as_name": "day" },
            { "column": "lines", "type": "int", "as_name": "total" }
        ]
    }));
    assert!(inner.numbered());
    assert_eq!(inner.hidden(), [TWIN_COLUMN]);
    let order = OrderKey::build(&columns()[..2], &sort("day", false), inner.hidden());

    let (sql, _) = wrap(&inner, &order, None, 100, &columns()[..2]);

    assert!(
        sql.contains(
            "FROM (SELECT __i.*, row_number() OVER (PARTITION BY __i.`day`, __i.`total`) \
             AS `__drilldown_twin` FROM (SELECT "
        ),
        "{sql}"
    );
    assert!(sql.contains(") AS __i) AS __m ORDER BY"), "{sql}");
    assert!(
        sql.contains("ifNull(toString(__m.`__drilldown_twin`), '') AS `__drilldown_tie2`"),
        "{sql}"
    );

    let grouped = grouped_over_a_table();
    assert!(!grouped.numbered());
    assert!(grouped.hidden().is_empty());
}
