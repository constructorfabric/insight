use chrono::{DateTime, TimeZone as _, Utc};
use serde_json::{Value, json};

use super::evaluate;
use crate::domain::alerts::condition::{Condition, Witness};
use crate::domain::alerts::scalar::{Outcome, UnknownReason};
use crate::domain::query::metric_query::{ColumnKind, RunResult};

const COLUMNS: [&str; 4] = ["team", "open", "done", "opened_at"];

fn kinds() -> Vec<(String, ColumnKind)> {
    [
        ("team", ColumnKind::Text),
        ("open", ColumnKind::Number),
        ("done", ColumnKind::Text),
        ("opened_at", ColumnKind::Date),
    ]
    .into_iter()
    .map(|(name, kind)| (name.to_owned(), kind))
    .collect()
}

fn rows(cells: &[[Value; 4]]) -> RunResult {
    RunResult {
        columns: COLUMNS.iter().map(|&name| name.to_owned()).collect(),
        rows: cells.iter().map(|row| row.to_vec()).collect(),
        percents: Vec::new(),
        undated: None,
        clock: None,
    }
}

fn condition(written: &Value) -> Condition {
    Condition::parse(written, &kinds()).unwrap_or_else(|error| panic!("{written}: {error}"))
}

fn checked_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 14, 12, 0, 0)
        .single()
        .unwrap_or_default()
}

fn row(team: &str, open: Value) -> [Value; 4] {
    [
        json!(team),
        open,
        json!(false),
        json!("2026-10-01 00:00:00"),
    ]
}

fn decided(outcome: &Outcome) -> Result<bool, UnknownReason> {
    match outcome {
        Outcome::Valid { breached, .. } => Ok(*breached),
        Outcome::Unknown(reason) => Err(*reason),
    }
}

#[test]
fn each_quantifier_decides_over_zero_one_and_many_rows() {
    let open_above_10 = json!({ "column": "open", "op": ">", "value": 10 });
    let with_rows = |rows: &str| json!({ "rows": rows, "where": open_above_10 });
    let count = |op: &str, value: u64| json!({ "rows": { "count": { "op": op, "value": value } }, "where": open_above_10 });

    let none_seen = rows(&[]);
    let one_high = rows(&[row("core", json!(12))]);
    let one_low = rows(&[row("core", json!(3))]);
    let mixed = rows(&[
        row("core", json!(12)),
        row("infra", json!(3)),
        row("data", json!(15)),
    ]);
    let all_high = rows(&[row("core", json!(12)), row("infra", json!(11))]);
    let all_low = rows(&[row("core", json!(1)), row("infra", json!(2))]);

    let cases: Vec<(&str, Value, &RunResult, Result<bool, UnknownReason>)> = vec![
        (
            "one over none",
            with_rows("one"),
            &none_seen,
            Err(UnknownReason::NoRows),
        ),
        (
            "one over one passing",
            with_rows("one"),
            &one_high,
            Ok(true),
        ),
        (
            "one over one failing",
            with_rows("one"),
            &one_low,
            Ok(false),
        ),
        (
            "one over many",
            with_rows("one"),
            &mixed,
            Err(UnknownReason::ManyRows),
        ),
        ("any over none", with_rows("any"), &none_seen, Ok(false)),
        ("any over mixed", with_rows("any"), &mixed, Ok(true)),
        (
            "any over all failing",
            with_rows("any"),
            &all_low,
            Ok(false),
        ),
        (
            "all over none",
            with_rows("all"),
            &none_seen,
            Err(UnknownReason::NoRows),
        ),
        ("all over mixed", with_rows("all"), &mixed, Ok(false)),
        (
            "all over all passing",
            with_rows("all"),
            &all_high,
            Ok(true),
        ),
        (
            "none over none",
            with_rows("none"),
            &none_seen,
            Err(UnknownReason::NoRows),
        ),
        ("none over mixed", with_rows("none"), &mixed, Ok(false)),
        (
            "none over all failing",
            with_rows("none"),
            &all_low,
            Ok(true),
        ),
        (
            "count over none compares zero",
            count("==", 0),
            &none_seen,
            Ok(true),
        ),
        ("count above, met", count(">", 1), &mixed, Ok(true)),
        ("count above, not met", count(">", 2), &mixed, Ok(false)),
        ("count is exactly", count("==", 2), &mixed, Ok(true)),
        ("count is not", count("!=", 2), &mixed, Ok(false)),
        ("count at most", count("<=", 1), &mixed, Ok(false)),
    ];

    for (case, written, result, expected) in cases {
        let outcome = evaluate(result, &condition(&written), &[], None, checked_at());
        assert_eq!(decided(&outcome), expected, "should decide: {case}");
    }
}

#[test]
fn an_unknown_cell_decides_only_what_it_cannot_change() {
    let null_open = rows(&[row("core", Value::Null)]);
    let cases = [
        (
            "alone, it is unknown",
            json!({ "column": "open", "op": ">", "value": 10 }),
            Err(UnknownReason::Null),
        ),
        (
            "inside all beside a false test, the group is false",
            json!({ "all": [
                { "column": "open", "op": ">", "value": 10 },
                { "column": "team", "op": "==", "value": "infra" },
            ] }),
            Ok(false),
        ),
        (
            "inside all beside a true test, the group is unknown",
            json!({ "all": [
                { "column": "open", "op": ">", "value": 10 },
                { "column": "team", "op": "==", "value": "core" },
            ] }),
            Err(UnknownReason::Null),
        ),
        (
            "inside any beside a true test, the group is true",
            json!({ "any": [
                { "column": "open", "op": ">", "value": 10 },
                { "column": "team", "op": "==", "value": "core" },
            ] }),
            Ok(true),
        ),
        (
            "not keeps it unknown",
            json!({ "not": { "column": "open", "op": ">", "value": 10 } }),
            Err(UnknownReason::Null),
        ),
    ];

    for (case, test, expected) in cases {
        let written = json!({ "rows": "one", "where": test });
        let outcome = evaluate(&null_open, &condition(&written), &[], None, checked_at());
        assert_eq!(decided(&outcome), expected, "{case}");
    }
}

#[test]
fn an_unknown_row_never_reads_as_a_clear_across_rows() {
    let open_above_10 = json!({ "column": "open", "op": ">", "value": 10 });
    let one_unknown = rows(&[row("core", json!(3)), row("infra", Value::Null)]);
    let unknown_and_passing = rows(&[row("core", json!(12)), row("infra", Value::Null)]);

    let cases = [
        (
            "any with no row passing",
            "any",
            &one_unknown,
            Err(UnknownReason::Null),
        ),
        (
            "any with one row passing",
            "any",
            &unknown_and_passing,
            Ok(true),
        ),
        ("all with one row failing", "all", &one_unknown, Ok(false)),
        (
            "none with one row passing",
            "none",
            &unknown_and_passing,
            Ok(false),
        ),
        (
            "none with no row passing",
            "none",
            &one_unknown,
            Err(UnknownReason::Null),
        ),
    ];

    for (case, quantifier, result, expected) in cases {
        let written = json!({ "rows": quantifier, "where": open_above_10 });
        let outcome = evaluate(result, &condition(&written), &[], None, checked_at());
        assert_eq!(decided(&outcome), expected, "{case}");
    }

    let count = |op: &str, value: u64| json!({ "rows": { "count": { "op": op, "value": value } }, "where": open_above_10 });
    let count_cases = [
        (
            "a count the unknown row cannot reach",
            count(">", 5),
            Ok(false),
        ),
        (
            "a count the unknown row could tip",
            count(">", 1),
            Err(UnknownReason::Null),
        ),
        (
            "a count already met whatever the unknown row is",
            count(">=", 1),
            Ok(true),
        ),
    ];
    for (case, written, expected) in count_cases {
        let outcome = evaluate(
            &unknown_and_passing,
            &condition(&written),
            &[],
            None,
            checked_at(),
        );
        assert_eq!(decided(&outcome), expected, "{case}");
    }
}

#[test]
fn a_cell_of_the_wrong_kind_is_unknown_with_its_reason() {
    let cases = [
        (
            "number test on text",
            json!({ "column": "open", "op": ">", "value": 1 }),
            json!("12"),
            UnknownReason::NonNumeric,
        ),
        (
            "wide integer against a float",
            json!({ "column": "open", "op": ">", "value": 1.5 }),
            json!(9_007_199_254_740_993_u64),
            UnknownReason::Incomparable,
        ),
    ];
    for (case, test, cell, reason) in cases {
        let result = rows(&[row("core", cell)]);
        let written = json!({ "rows": "one", "where": test });
        let outcome = evaluate(&result, &condition(&written), &[], None, checked_at());
        assert_eq!(decided(&outcome), Err(reason), "{case}");
    }

    let text_on_number = rows(&[[json!(7), json!(1), json!(false), json!("2026-10-01")]]);
    let written = json!({ "rows": "one", "where": { "column": "team", "op": "==", "value": "7" } });
    assert_eq!(
        decided(&evaluate(
            &text_on_number,
            &condition(&written),
            &[],
            None,
            checked_at()
        )),
        Err(UnknownReason::NonText)
    );

    let not_a_date = rows(&[[json!("core"), json!(1), json!(false), json!("last week")]]);
    let written = json!({ "rows": "one", "where": { "column": "opened_at", "op": "older_than", "value": "P1D" } });
    assert_eq!(
        decided(&evaluate(
            &not_a_date,
            &condition(&written),
            &[],
            None,
            checked_at()
        )),
        Err(UnknownReason::NonDate)
    );
}

#[test]
fn text_compares_exactly_and_a_boolean_reads_as_its_word() {
    let result = rows(&[[
        json!("core-api"),
        json!(1),
        json!(true),
        json!("2026-10-01"),
    ]]);
    let cases = [
        (
            json!({ "column": "team", "op": "==", "value": "core-api" }),
            true,
        ),
        (
            json!({ "column": "team", "op": "==", "value": "Core-API" }),
            false,
        ),
        (
            json!({ "column": "team", "op": "!=", "value": "core" }),
            true,
        ),
        (
            json!({ "column": "team", "op": "contains", "value": "re-a" }),
            true,
        ),
        (
            json!({ "column": "team", "op": "starts_with", "value": "core" }),
            true,
        ),
        (
            json!({ "column": "team", "op": "ends_with", "value": "api" }),
            true,
        ),
        (
            json!({ "column": "team", "op": "ends_with", "value": "core" }),
            false,
        ),
        (
            json!({ "column": "team", "op": "in", "value": ["infra", "core-api"] }),
            true,
        ),
        (
            json!({ "column": "team", "op": "not_in", "value": ["infra", "core-api"] }),
            false,
        ),
        (
            json!({ "column": "done", "op": "==", "value": "true" }),
            true,
        ),
    ];

    for (test, expected) in cases {
        let written = json!({ "rows": "one", "where": test });
        let outcome = evaluate(&result, &condition(&written), &[], None, checked_at());
        assert_eq!(decided(&outcome), Ok(expected), "for {test}");
    }
}

#[test]
fn a_date_is_read_in_the_warehouse_forms_and_an_age_is_measured_from_the_check() {
    let at = |cell: &str| rows(&[[json!("core"), json!(1), json!(false), json!(cell)]]);
    let test = |op: &str, value: Value| json!({ "rows": "one", "where": { "column": "opened_at", "op": op, "value": value } });

    let cases = [
        (
            "a day and a minute old is older than a day",
            at("2026-10-13 11:59:00"),
            test("older_than", json!("P1D")),
            true,
        ),
        (
            "exactly a day old is not older than a day",
            at("2026-10-13 12:00:00.000"),
            test("older_than", json!("P1D")),
            false,
        ),
        (
            "exactly a day old is newer than a day",
            at("2026-10-13T12:00:00Z"),
            test("newer_than", json!("P1D")),
            true,
        ),
        (
            "five hours old is newer than six",
            at("2026-10-14T09:00:00+02:00"),
            test("newer_than", json!("PT6H")),
            true,
        ),
        (
            "a bare date is its midnight",
            at("2026-10-01"),
            test("before", json!("2026-10-01T00:00:01Z")),
            true,
        ),
        (
            "between includes both ends",
            at("2026-10-01 00:00:00"),
            test(
                "between",
                json!(["2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z"]),
            ),
            true,
        ),
        (
            "after is strict",
            at("2026-10-01 00:00:00"),
            test("after", json!("2026-10-01T00:00:00Z")),
            false,
        ),
    ];

    for (case, result, written, expected) in cases {
        let outcome = evaluate(&result, &condition(&written), &[], None, checked_at());
        assert_eq!(decided(&outcome), Ok(expected), "{case}");
    }
}

#[test]
fn a_number_between_includes_both_ends() {
    let written =
        json!({ "rows": "one", "where": { "column": "open", "op": "between", "value": [5, 10] } });
    for (open, expected) in [(4, false), (5, true), (10, true), (11, false)] {
        let result = rows(&[row("core", json!(open))]);
        let outcome = evaluate(&result, &condition(&written), &[], None, checked_at());
        assert_eq!(decided(&outcome), Ok(expected), "for {open}");
    }
}

#[test]
fn the_witness_shows_the_group_and_condition_columns_of_at_most_five_passing_rows() {
    let many: Vec<[Value; 4]> = (0..8)
        .map(|index| row(&format!("team-{index}"), json!(20 + index)))
        .collect();
    let mut with_a_failing_row = many.clone();
    with_a_failing_row.insert(1, row("quiet", json!(0)));
    let result = rows(&with_a_failing_row);
    let written = json!({ "rows": "any", "where": { "column": "open", "op": ">", "value": 10 } });

    let outcome = evaluate(
        &result,
        &condition(&written),
        &["team".to_owned()],
        None,
        checked_at(),
    );

    let Outcome::Valid {
        breached: true,
        witness,
    } = outcome
    else {
        panic!("should breach: {outcome:?}");
    };
    assert_eq!((witness.matched, witness.answered), (8, 9));
    assert_eq!(witness.columns, ["team", "open"]);
    assert_eq!(
        witness.rows,
        (0..5)
            .map(|index| vec![json!(format!("team-{index}")), json!(20 + index)])
            .collect::<Vec<_>>(),
        "the failing row is left out and the rest are cut at five"
    );
}

#[test]
fn a_one_row_witness_keeps_its_row_whatever_the_test_found() {
    let result = rows(&[row("core", json!(3))]);
    let written = json!({ "rows": "one", "where": { "column": "open", "op": ">", "value": 10 } });

    assert_eq!(
        evaluate(&result, &condition(&written), &[], None, checked_at()),
        Outcome::Valid {
            breached: false,
            witness: Witness::one_value("open", json!(3), false),
        }
    );
}

#[test]
fn a_long_text_cell_is_cut_in_the_witness() {
    let long = "x".repeat(400);
    let result = rows(&[[json!(long), json!(12), json!(false), json!("2026-10-01")]]);
    let written =
        json!({ "rows": "any", "where": { "column": "team", "op": "starts_with", "value": "x" } });

    let outcome = evaluate(&result, &condition(&written), &[], None, checked_at());

    let Some(witness) = outcome.witness() else {
        panic!("should be valid: {outcome:?}");
    };
    assert_eq!(witness.rows[0][0], json!("x".repeat(256)));
}

#[test]
fn a_column_the_result_lacks_is_unknown_even_over_no_rows() {
    let result = RunResult {
        columns: vec!["team".to_owned()],
        rows: Vec::new(),
        percents: Vec::new(),
        undated: None,
        clock: None,
    };
    let written = json!({ "rows": "any", "where": { "column": "open", "op": ">", "value": 10 } });

    assert_eq!(
        evaluate(&result, &condition(&written), &[], None, checked_at()),
        Outcome::Unknown(UnknownReason::ColumnMissing)
    );
}

#[test]
fn a_run_cut_at_the_row_bound_decides_only_what_the_missing_rows_cannot_change() {
    let open_above_10 = json!({ "column": "open", "op": ">", "value": 10 });
    let with_rows = |rows: &str| json!({ "rows": rows, "where": open_above_10 });
    let count = |op: &str, value: u64| json!({ "rows": { "count": { "op": op, "value": value } }, "where": open_above_10 });
    let passing = rows(&[row("core", json!(12)), row("infra", json!(3))]);
    let failing = rows(&[row("core", json!(1)), row("infra", json!(3))]);

    let cases: Vec<(&str, Value, &RunResult, Result<bool, UnknownReason>)> = vec![
        ("any already met", with_rows("any"), &passing, Ok(true)),
        (
            "any not yet met",
            with_rows("any"),
            &failing,
            Err(UnknownReason::RowLimit),
        ),
        ("all already failed", with_rows("all"), &passing, Ok(false)),
        (
            "none already failed",
            with_rows("none"),
            &passing,
            Ok(false),
        ),
        (
            "none not yet failed",
            with_rows("none"),
            &failing,
            Err(UnknownReason::RowLimit),
        ),
        ("count already above", count(">=", 1), &passing, Ok(true)),
        (
            "count not yet above",
            count(">", 5),
            &passing,
            Err(UnknownReason::RowLimit),
        ),
        (
            "count already past at most",
            count("<=", 0),
            &passing,
            Ok(false),
        ),
    ];

    for (case, written, result, expected) in cases {
        let outcome = evaluate(result, &condition(&written), &[], Some(2), checked_at());
        assert_eq!(decided(&outcome), expected, "should decide: {case}");
    }

    let short_of_the_bound = evaluate(
        &failing,
        &condition(&with_rows("any")),
        &[],
        Some(3),
        checked_at(),
    );
    assert_eq!(
        decided(&short_of_the_bound),
        Ok(false),
        "a run short of the bound is whole"
    );
}

#[test]
fn a_witness_keeps_wide_integers_as_digits_and_lists_as_cut_text() {
    let result = rows(&[[
        json!(["core", "infra"]),
        json!(9_007_199_254_740_993_u64),
        json!(false),
        json!("2026-10-01"),
    ]]);
    let written = json!({ "rows": "any", "where": { "column": "open", "op": ">", "value": 1 } });

    let outcome = evaluate(
        &result,
        &condition(&written),
        &["team".to_owned()],
        None,
        checked_at(),
    );

    let Some(witness) = outcome.witness() else {
        panic!("should be valid: {outcome:?}");
    };
    assert_eq!(
        witness.rows,
        [[json!("[\"core\",\"infra\"]"), json!("9007199254740993")]]
    );
}
