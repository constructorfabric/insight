use serde_json::{Map, Value, json};

use super::*;
use crate::domain::query::metric_query::ColumnKind;

fn columns() -> Vec<(String, ColumnKind)> {
    [
        ("open", ColumnKind::Number),
        ("stale", ColumnKind::Number),
        ("team", ColumnKind::Text),
        ("opened_at", ColumnKind::Date),
    ]
    .into_iter()
    .map(|(name, kind)| (name.to_owned(), kind))
    .collect()
}

fn parsed(value: &Value) -> Condition {
    Condition::parse(value, &columns())
        .unwrap_or_else(|error| panic!("should accept {value}: {error}"))
}

fn refused(value: &Value) -> ConditionError {
    match Condition::parse(value, &columns()) {
        Ok(condition) => panic!("should refuse {value}, read {condition:?}"),
        Err(error) => error,
    }
}

fn leaf(column: &str, op: &str, value: Value) -> Value {
    let mut test = Map::new();
    test.insert("column".to_owned(), column.into());
    test.insert("op".to_owned(), op.into());
    test.insert("value".to_owned(), value);

    Value::Object(test)
}

fn one(test: Value) -> Value {
    let mut condition = Map::new();
    condition.insert("rows".to_owned(), "one".into());
    condition.insert("where".to_owned(), test);

    Value::Object(condition)
}

#[test]
fn every_operator_applies_to_its_own_kind_of_column() {
    let cases = [
        leaf("open", ">", json!(10)),
        leaf("open", ">=", json!(10)),
        leaf("open", "<", json!(10.5)),
        leaf("open", "<=", json!("9007199254740993")),
        leaf("open", "==", json!(0)),
        leaf("open", "!=", json!(-1)),
        leaf("open", "between", json!([1, 5])),
        leaf("team", "==", json!("core")),
        leaf("team", "!=", json!("core")),
        leaf("team", "contains", json!("or")),
        leaf("team", "starts_with", json!("co")),
        leaf("team", "ends_with", json!("re")),
        leaf("team", "in", json!(["core", "infra"])),
        leaf("team", "not_in", json!(["core"])),
        leaf("opened_at", "before", json!("2026-10-01T00:00:00Z")),
        leaf("opened_at", "after", json!("2026-10-01T00:00:00+02:00")),
        leaf(
            "opened_at",
            "between",
            json!(["2026-09-01T00:00:00Z", "2026-10-01T00:00:00Z"]),
        ),
        leaf("opened_at", "older_than", json!("P7D")),
        leaf("opened_at", "newer_than", json!("PT30M")),
    ];

    for case in cases {
        parsed(&one(case));
    }
}

#[test]
fn an_operator_is_refused_on_a_column_of_another_kind() {
    let cases = [
        (leaf("open", "contains", json!("1")), "number"),
        (
            leaf("open", "before", json!("2026-10-01T00:00:00Z")),
            "number",
        ),
        (leaf("team", ">", json!(1)), "text"),
        (leaf("team", "older_than", json!("P7D")), "text"),
        (leaf("opened_at", ">", json!(1)), "date"),
        (leaf("opened_at", "contains", json!("x")), "date"),
    ];

    for (case, kind) in cases {
        let error = refused(&one(case.clone()));
        assert_eq!(error.path, "where.op", "should name the operator: {case}");
        assert!(
            matches!(&error.problem, Problem::Operator { kind: named, .. } if *named == kind),
            "should say the column is {kind}: {case} gave {error}"
        );
    }
}

#[test]
fn a_refusal_names_the_path_of_what_is_wrong() {
    let nested = json!({
        "rows": "any",
        "where": { "all": [
            leaf("open", ">", json!(10)),
            { "any": [ leaf("team", "==", json!("core")), leaf("stale", ">", json!("three")) ] }
        ] }
    });
    let cases: Vec<(Value, &str, Problem)> = vec![
        (nested, "where.all[1].any[1].value", Problem::NumberValue),
        (json!({ "rows": "one" }), "where", Problem::Missing),
        (
            json!({ "rows": "some", "where": leaf("open", ">", json!(1)) }),
            "rows",
            Problem::Rows,
        ),
        (
            json!({ "rows": "one", "where": leaf("open", ">", json!(1)), "extra": 1 }),
            "extra",
            Problem::UnknownKey,
        ),
        (
            one(leaf("closed", ">", json!(1))),
            "where.column",
            Problem::NoSuchColumn("closed".to_owned()),
        ),
        (one(json!({ "all": [] })), "where.all", Problem::EmptyGroup),
        (
            one(json!({ "all": [leaf("open", ">", json!(1))], "any": [] })),
            "where.any",
            Problem::UnknownKey,
        ),
        (
            one(json!({ "column": "open", "op": ">" })),
            "where.value",
            Problem::Missing,
        ),
        (one(json!(["open"])), "where", Problem::Node),
        (
            one(leaf("open", "between", json!([5, 1]))),
            "where.value",
            Problem::BoundsOrder,
        ),
        (
            one(leaf("open", "between", json!(["9007199254740993", 1.5]))),
            "where.value",
            Problem::BoundsIncomparable,
        ),
        (
            one(leaf("open", "between", json!([1]))),
            "where.value",
            Problem::Pair,
        ),
        (
            one(leaf(
                "opened_at",
                "between",
                json!(["2026-10-02T00:00:00Z", "2026-10-01T00:00:00Z"]),
            )),
            "where.value",
            Problem::BoundsOrder,
        ),
        (
            one(leaf("opened_at", "before", json!("2026-10-01"))),
            "where.value",
            Problem::DateValue,
        ),
        (
            one(leaf("team", "in", json!(["core", 3]))),
            "where.value[1]",
            Problem::TextValue,
        ),
        (
            one(leaf("team", "in", json!([]))),
            "where.value",
            Problem::ListValue,
        ),
        (
            json!({ "rows": { "count": { "op": "~", "value": 3 } }, "where": leaf("open", ">", json!(1)) }),
            "rows.count.op",
            Problem::Operator {
                operator: "~".to_owned(),
                kind: "row count",
                allowed: "`>`, `>=`, `<`, `<=`, `==`, `!=`",
            },
        ),
        (
            json!({ "rows": { "count": { "op": ">", "value": -1 } }, "where": leaf("open", ">", json!(1)) }),
            "rows.count.value",
            Problem::Count,
        ),
    ];

    for (case, path, problem) in cases {
        let error = refused(&case);
        assert_eq!(
            (error.path.as_str(), &error.problem),
            (path, &problem),
            "for {case}"
        );
    }
}

#[test]
fn a_timestamp_whose_utc_year_cannot_be_written_back_is_refused() {
    let cases = [
        ("9999-12-31T23:59:59-10:00", false),
        ("0000-01-01T00:00:00+10:00", false),
        ("9999-12-31T23:59:59Z", true),
        ("0000-01-01T00:00:00Z", true),
    ];

    for (moment, accepted) in cases {
        let written = one(leaf("opened_at", "before", json!(moment)));
        match Condition::parse(&written, &columns()) {
            Ok(_) => assert!(accepted, "should refuse {moment}"),
            Err(error) => {
                assert!(!accepted, "should accept {moment}: {error}");
                assert_eq!(error.problem, Problem::DateValue, "for {moment}");
            }
        }
    }
}

#[test]
fn every_accepted_condition_reads_back_from_how_a_row_stores_it() {
    let cases = [
        leaf("open", ">", json!(10)),
        leaf("open", "<=", json!("9007199254740993")),
        leaf("open", "between", json!([-1.5, 1e300])),
        leaf("team", "in", json!(["core", "infra"])),
        leaf("team", "starts_with", json!("")),
        leaf(
            "opened_at",
            "between",
            json!(["0000-01-01T00:00:00Z", "9999-12-31T23:59:59.999Z"]),
        ),
        leaf(
            "opened_at",
            "after",
            json!("2026-10-01T02:00:00.123456+02:00"),
        ),
        leaf("opened_at", "newer_than", json!("PT30M")),
        json!({ "not": { "any": [ { "all": [leaf("open", ">", json!(1))] } ] } }),
    ];
    let quantifiers = [
        json!("one"),
        json!("any"),
        json!("all"),
        json!("none"),
        json!({ "count": { "op": "!=", "value": 0 } }),
    ];

    for test in &cases {
        for rows in &quantifiers {
            let written = json!({ "rows": rows, "where": test });
            let condition = parsed(&written);
            assert_eq!(
                Condition::from_stored(&condition.to_stored()),
                Some(condition),
                "should read back what it stores: {written}"
            );
        }
    }
}

#[test]
fn durations_are_whole_minutes_hours_days_or_weeks_up_to_a_century() {
    let accepted = ["PT1M", "PT6H", "P7D", "P2W", "P36500D", "P5214W"];
    let refused_ages = [
        "P0D", "P1M", "PT1D", "P1.5D", "P-1D", "7D", "P", "P36501D", "P5215W", "P1Y",
    ];

    for age in accepted {
        parsed(&one(leaf("opened_at", "older_than", json!(age))));
    }
    for age in refused_ages {
        let error = refused(&one(leaf("opened_at", "older_than", json!(age))));
        assert_eq!(
            error.problem,
            Problem::AgeValue,
            "should refuse duration {age}"
        );
    }
}

#[test]
fn bounds_are_accepted_at_their_limit_and_refused_one_past_it() {
    let nested = |depth: usize| {
        let mut node = leaf("open", ">", json!(1));
        for _ in 0..depth {
            node = json!({ "all": [node] });
        }
        one(node)
    };
    let tests = |count: usize| one(json!({ "any": vec![leaf("open", ">", json!(1)); count] }));
    let list = |count: usize| one(leaf("team", "in", json!(vec!["core"; count])));
    let text = |chars: usize| one(leaf("team", "==", json!("x".repeat(chars))));

    let cases: Vec<(&str, Value, Value, Problem)> = vec![
        (
            "group depth",
            nested(MAX_GROUP_DEPTH),
            nested(MAX_GROUP_DEPTH + 1),
            Problem::TooDeep,
        ),
        (
            "tests",
            tests(MAX_LEAVES),
            tests(MAX_LEAVES + 1),
            Problem::TooManyTests,
        ),
        (
            "list values",
            list(MAX_LIST_VALUES),
            list(MAX_LIST_VALUES + 1),
            Problem::ListValue,
        ),
        (
            "text length",
            text(MAX_TEXT_CHARS),
            text(MAX_TEXT_CHARS + 1),
            Problem::TextValue,
        ),
    ];

    for (bound, at_limit, past_limit, problem) in cases {
        parsed(&at_limit);
        assert_eq!(
            refused(&past_limit).problem,
            problem,
            "should refuse past the {bound} bound"
        );
    }
}

#[test]
fn a_not_counts_toward_the_group_depth() {
    let three = one(json!({ "not": { "any": [ { "all": [leaf("open", ">", json!(1))] } ] } }));
    let four =
        one(json!({ "not": { "not": { "any": [ { "all": [leaf("open", ">", json!(1))] } ] } } }));

    parsed(&three);
    assert_eq!(refused(&four).problem, Problem::TooDeep);
}

#[test]
fn a_condition_reads_back_as_the_json_it_was_written_in() {
    let cases = [
        one(leaf("open", ">", json!(10))),
        one(leaf("open", "<=", json!("9007199254740993"))),
        json!({
            "rows": { "count": { "op": ">=", "value": 3 } },
            "where": { "all": [
                leaf("open", "between", json!([1, 2.5])),
                { "not": leaf("team", "in", json!(["core", "infra"])) },
                { "any": [
                    leaf("opened_at", "between", json!(["2026-09-01T00:00:00Z", "2026-10-01T12:30:00.500Z"])),
                    leaf("opened_at", "older_than", json!("P2W")),
                    leaf("team", "ends_with", json!("ops")),
                ] },
            ] },
        }),
    ];

    for case in cases {
        assert_eq!(
            parsed(&case).to_json(),
            case,
            "should write back what was read"
        );
    }
}

#[test]
fn a_timestamp_with_an_offset_is_written_back_in_utc() {
    let written = parsed(&one(leaf(
        "opened_at",
        "after",
        json!("2026-10-01T02:00:00+02:00"),
    )))
    .to_json();

    assert_eq!(written["where"]["value"], json!("2026-10-01T00:00:00Z"));
}

#[test]
fn a_condition_reads_in_words() {
    let cases = [
        (one(leaf("open", ">", json!(10))), "open above 10"),
        (
            one(leaf("open", "between", json!([0.25, 1]))),
            "open between 0.25 and 1",
        ),
        (
            json!({ "rows": "any", "where": { "all": [
                leaf("open", ">", json!(10)),
                { "any": [ leaf("team", "==", json!("core")), leaf("stale", ">", json!(3)) ] },
            ] } }),
            "any row: open above 10 and (team is core or stale above 3)",
        ),
        (
            json!({ "rows": "none", "where": { "not": leaf("team", "in", json!(["core", "infra"])) } }),
            "no row: not (team is one of core, infra)",
        ),
        (
            json!({ "rows": { "count": { "op": ">", "value": 3 } }, "where": leaf("opened_at", "older_than", json!("P1D")) }),
            "number of matching rows above 3: opened_at older than 1 day",
        ),
        (
            json!({ "rows": "all", "where": leaf("opened_at", "before", json!("2026-10-01T09:05:00Z")) }),
            "every row: opened_at before 2026-10-01 09:05 UTC",
        ),
    ];

    for (case, words) in cases {
        assert_eq!(parsed(&case).to_string(), words, "for {case}");
    }
}

#[test]
fn the_columns_a_condition_reads_are_listed_once_in_the_order_named() {
    let condition = parsed(&one(json!({ "any": [
        leaf("team", "==", json!("core")),
        { "all": [ leaf("open", ">", json!(1)), leaf("team", "!=", json!("infra")) ] },
    ] })));

    assert_eq!(condition.columns(), ["team", "open"]);
}
