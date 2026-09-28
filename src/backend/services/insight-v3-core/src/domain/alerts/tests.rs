use std::collections::BTreeMap;

use serde_json::json;

use super::rule::{Condition, Operator, RuleSpec};
use super::scalar::classify;
use super::*;
use crate::domain::query::metric_query::RunResult;

fn result(columns: &[&str], rows: &[&[serde_json::Value]]) -> RunResult {
    RunResult {
        columns: columns.iter().map(|&name| name.to_owned()).collect(),
        rows: rows.iter().map(|row| row.to_vec()).collect(),
        percents: Vec::new(),
        undated: None,
        clock: None,
    }
}

fn above(threshold: &serde_json::Value) -> Condition {
    Condition {
        operator: Operator::Gt,
        threshold: Number::parse(threshold).unwrap_or_else(|| panic!("{threshold} is a number")),
    }
}

#[test]
fn exactly_one_row_and_the_named_column_make_a_valid_check() {
    let cases: Vec<(&str, RunResult, Outcome)> = vec![
        (
            "one row above",
            result(&["total"], &[&[json!(12)]]),
            Outcome::Valid {
                value: Number::Int(12),
                breached: true,
            },
        ),
        (
            "one row at the threshold is not above it",
            result(&["total"], &[&[json!(10)]]),
            Outcome::Valid {
                value: Number::Int(10),
                breached: false,
            },
        ),
        (
            "no rows",
            result(&["total"], &[]),
            Outcome::Unknown(UnknownReason::NoRows),
        ),
        (
            "two rows",
            result(&["total"], &[&[json!(1)], &[json!(2)]]),
            Outcome::Unknown(UnknownReason::ManyRows),
        ),
        (
            "column absent",
            result(&["other"], &[&[json!(1)]]),
            Outcome::Unknown(UnknownReason::ColumnMissing),
        ),
        (
            "null cell",
            result(&["total"], &[&[serde_json::Value::Null]]),
            Outcome::Unknown(UnknownReason::Null),
        ),
        (
            "text cell",
            result(&["total"], &[&[json!("12")]]),
            Outcome::Unknown(UnknownReason::NonNumeric),
        ),
        (
            "float above an integer threshold",
            result(&["total"], &[&[json!(10.5)]]),
            Outcome::Valid {
                value: Number::Float(10.5),
                breached: true,
            },
        ),
    ];

    for (case, rows, expected) in cases {
        assert_eq!(
            classify(&rows, "total", &above(&json!(10))),
            expected,
            "should classify: {case}"
        );
    }
}

#[test]
fn an_integer_too_wide_for_a_float_is_not_compared_with_one() {
    let wide = result(&["total"], &[&[json!(9_007_199_254_740_993_u64)]]);

    assert_eq!(
        classify(&wide, "total", &above(&json!(1.5))),
        Outcome::Unknown(UnknownReason::Incomparable)
    );
    assert_eq!(
        classify(&wide, "total", &above(&json!(1))),
        Outcome::Valid {
            value: Number::Int(9_007_199_254_740_993),
            breached: true,
        }
    );
}

#[test]
fn every_operator_reads_as_written() {
    let cases = [
        (Operator::Gt, 11, true),
        (Operator::Gt, 10, false),
        (Operator::Ge, 10, true),
        (Operator::Ge, 9, false),
        (Operator::Lt, 9, true),
        (Operator::Lt, 10, false),
        (Operator::Le, 10, true),
        (Operator::Le, 11, false),
    ];

    for (operator, value, expected) in cases {
        let condition = Condition {
            operator,
            threshold: Number::Int(10),
        };

        assert_eq!(
            condition.holds(Number::Int(value)),
            Some(expected),
            "should decide: {value} {}",
            operator.as_str()
        );
    }
}

#[test]
fn a_notification_is_owed_on_the_first_breach_and_again_only_after_a_valid_clear() {
    let breach = Outcome::Valid {
        value: Number::Int(1),
        breached: true,
    };
    let clear = Outcome::Valid {
        value: Number::Int(0),
        breached: false,
    };
    let unknown = Outcome::Unknown(UnknownReason::NoRows);

    let cases = [
        (
            "first breach from nothing",
            None,
            &breach,
            Transition::Notify,
        ),
        (
            "first breach after a clear",
            Some(false),
            &breach,
            Transition::Notify,
        ),
        ("still breached", Some(true), &breach, Transition::Silent),
        ("clears", Some(true), &clear, Transition::Silent),
        (
            "unknown during a breach",
            Some(true),
            &unknown,
            Transition::Silent,
        ),
        (
            "unknown before anything",
            None,
            &unknown,
            Transition::Silent,
        ),
    ];

    for (case, previous, outcome, expected) in cases {
        assert_eq!(
            transition(previous, outcome),
            expected,
            "should decide: {case}"
        );
    }
}

#[test]
fn an_unknown_check_leaves_the_last_valid_finding_as_it_was() {
    let unknown = Outcome::Unknown(UnknownReason::Timeout);
    let clear = Outcome::Valid {
        value: Number::Int(0),
        breached: false,
    };

    assert_eq!(last_valid_breached(Some(true), &unknown), Some(true));
    assert_eq!(last_valid_breached(None, &unknown), None);
    assert_eq!(last_valid_breached(Some(true), &clear), Some(false));
}

#[test]
fn a_breach_that_never_clears_is_notified_once_across_many_checks() {
    let breach = Outcome::Valid {
        value: Number::Int(5),
        breached: true,
    };
    let mut previous = None;
    let mut notified = 0;

    for _ in 0..5 {
        if transition(previous, &breach) == Transition::Notify {
            notified += 1;
        }
        previous = last_valid_breached(previous, &breach);
    }

    assert_eq!(notified, 1);
}

fn destinations() -> Destinations {
    Destinations::new(BTreeMap::from([("ops".to_owned(), "discord".to_owned())]))
}

fn draft(edit: impl FnOnce(&mut serde_json::Value)) -> RuleDraft {
    let mut body = json!({
        "metric": "prs-open",
        "column": "total",
        "operator": ">",
        "threshold": 10,
        "interval_secs": 300,
        "destination": "ops"
    });
    edit(&mut body);

    serde_json::from_value(body).unwrap_or_else(|error| panic!("the draft parses: {error}"))
}

#[test]
fn a_draft_is_checked_against_the_bounds_before_it_is_a_rule() {
    let limits = Limits::default();
    let cases: Vec<(&str, RuleDraft, RuleError)> = vec![
        (
            "metric name",
            draft(|body| body["metric"] = json!("not a name!")),
            RuleError::Metric,
        ),
        (
            "column",
            draft(|body| body["column"] = json!("to-tal")),
            RuleError::Column,
        ),
        (
            "threshold text",
            draft(|body| body["threshold"] = json!("10")),
            RuleError::Threshold,
        ),
        (
            "interval below the floor",
            draft(|body| body["interval_secs"] = json!(1)),
            RuleError::Interval {
                min: limits.min_interval_secs,
                max: limits.max_interval_secs,
            },
        ),
        (
            "unknown destination",
            draft(|body| body["destination"] = json!("nowhere")),
            RuleError::Destination("nowhere".to_owned()),
        ),
    ];

    for (case, draft, expected) in cases {
        assert_eq!(
            RuleSpec::parse(&draft, limits, &destinations()).err(),
            Some(expected),
            "should refuse: {case}"
        );
    }

    let ok = RuleSpec::parse(&draft(|_| {}), limits, &destinations());
    assert!(ok.is_ok(), "a whole draft is a rule: {ok:?}");
}

#[test]
fn a_range_is_read_as_a_run_would_read_it() {
    let limits = Limits::default();

    let bad = draft(|body| body["range"] = json!("yesterday"));
    assert!(matches!(
        RuleSpec::parse(&bad, limits, &destinations()),
        Err(RuleError::Range(_))
    ));

    let good = draft(|body| body["range"] = json!("P7D"));
    let spec = RuleSpec::parse(&good, limits, &destinations())
        .unwrap_or_else(|error| panic!("P7D is a range: {error}"));
    let window = spec
        .window()
        .unwrap_or_else(|error| panic!("the window parses: {error}"));
    assert!(window.is_ranged());
}

#[test]
fn a_draft_names_only_the_fields_a_rule_has() {
    let extra = json!({
        "metric": "m", "column": "c", "operator": ">", "threshold": 1,
        "interval_secs": 60, "destination": "ops", "colour": "red"
    });

    assert!(serde_json::from_value::<RuleDraft>(extra).is_err());
}

#[test]
fn a_number_survives_the_row_it_is_stored_in() {
    for number in [
        Number::Int(0),
        Number::Int(-7),
        Number::Int(i128::from(u64::MAX)),
        Number::Float(0.1),
        Number::Float(-2.5e300),
    ] {
        assert_eq!(
            Number::from_stored(&number.to_stored()),
            Some(number),
            "should round-trip: {number}"
        );
    }
}
