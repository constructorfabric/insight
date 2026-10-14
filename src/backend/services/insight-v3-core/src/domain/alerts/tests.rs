use std::collections::BTreeMap;

use serde_json::json;

use chrono::{DateTime, Utc};

use super::condition::{Condition, ConditionError, Problem, Witness, evaluate};
use super::rule::RuleSpec;
use super::*;
use crate::domain::metric_run::Columns;
use crate::domain::query::metric_query::{ColumnKind, RunResult};

fn result(columns: &[&str], rows: &[&[serde_json::Value]]) -> RunResult {
    RunResult {
        columns: columns.iter().map(|&name| name.to_owned()).collect(),
        rows: rows.iter().map(|row| row.to_vec()).collect(),
        percents: Vec::new(),
        undated: None,
        clock: None,
    }
}

fn columns() -> Vec<(String, ColumnKind)> {
    vec![("total".to_owned(), ColumnKind::Number)]
}

/// What the metric answers, as a rule is checked against it.
fn metric_columns() -> Columns {
    Columns {
        kinds: columns(),
        groups: Vec::new(),
        imposed_limit: None,
    }
}

fn one_total(op: &str, threshold: &serde_json::Value) -> Condition {
    let written =
        json!({ "rows": "one", "where": { "column": "total", "op": op, "value": threshold } });

    Condition::parse(&written, &columns()).unwrap_or_else(|error| panic!("{written}: {error}"))
}

fn above(threshold: &serde_json::Value) -> Condition {
    one_total(">", threshold)
}

fn now() -> DateTime<Utc> {
    DateTime::UNIX_EPOCH
}

/// A valid one-row check of `total`, and the value it read.
fn seen(breached: bool, value: serde_json::Value) -> Outcome {
    Outcome::Valid {
        breached,
        witness: Witness {
            matched: u64::from(breached),
            answered: 1,
            columns: vec!["total".to_owned()],
            rows: vec![vec![value]],
        },
    }
}

#[test]
fn exactly_one_row_and_the_named_column_make_a_valid_check() {
    let cases: Vec<(&str, RunResult, Outcome)> = vec![
        (
            "one row above",
            result(&["total"], &[&[json!(12)]]),
            seen(true, json!(12)),
        ),
        (
            "one row at the threshold is not above it",
            result(&["total"], &[&[json!(10)]]),
            seen(false, json!(10)),
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
            seen(true, json!(10.5)),
        ),
    ];

    for (case, rows, expected) in cases {
        assert_eq!(
            evaluate(&rows, &above(&json!(10)), &[], &[], None, now()),
            expected,
            "should classify: {case}"
        );
    }
}

#[test]
fn an_integer_too_wide_for_a_float_is_not_compared_with_one() {
    let wide = result(&["total"], &[&[json!(9_007_199_254_740_993_u64)]]);

    assert_eq!(
        evaluate(&wide, &above(&json!(1.5)), &[], &[], None, now()),
        Outcome::Unknown(UnknownReason::Incomparable)
    );
    assert_eq!(
        evaluate(&wide, &above(&json!(1)), &[], &[], None, now()),
        seen(true, json!("9007199254740993"))
    );

    let at_the_bound = result(&["total"], &[&[json!(9_007_199_254_740_992_u64)]]);
    assert_eq!(
        evaluate(&at_the_bound, &above(&json!(1.5)), &[], &[], None, now()),
        seen(true, json!("9007199254740992")),
        "2^53 itself converts exactly"
    );
}

#[test]
fn an_integer_is_a_json_number_only_while_javascript_holds_it_exactly() {
    let safe = 9_007_199_254_740_991_i128;
    let cases = [
        (
            "2^53 - 1",
            Number::Int(safe),
            json!(9_007_199_254_740_991_i64),
        ),
        (
            "-(2^53 - 1)",
            Number::Int(-safe),
            json!(-9_007_199_254_740_991_i64),
        ),
        ("2^53", Number::Int(safe + 1), json!("9007199254740992")),
        ("-2^53", Number::Int(-safe - 1), json!("-9007199254740992")),
        (
            "i64::MAX",
            Number::Int(i128::from(i64::MAX)),
            json!(i64::MAX.to_string()),
        ),
        (
            "i64::MIN",
            Number::Int(i128::from(i64::MIN)),
            json!(i64::MIN.to_string()),
        ),
        (
            "u64::MAX",
            Number::Int(i128::from(u64::MAX)),
            json!(u64::MAX.to_string()),
        ),
        ("a float", Number::Float(2.5), json!(2.5)),
    ];

    for (case, number, expected) in cases {
        let shown = number.to_json();
        assert_eq!(shown, expected, "should show: {case}");
        assert_eq!(
            Number::parse_written(&shown),
            Some(number),
            "should take back what it showed: {case}"
        );
    }
}

#[test]
fn a_written_number_is_a_json_number_or_an_integer_in_digits_and_nothing_else() {
    let accepted = [
        (json!(10), Number::Int(10)),
        (json!(-7), Number::Int(-7)),
        (json!(1.5), Number::Float(1.5)),
        (json!("0"), Number::Int(0)),
        (
            json!("-9007199254740993"),
            Number::Int(-9_007_199_254_740_993),
        ),
        (
            json!("18446744073709551615"),
            Number::Int(i128::from(u64::MAX)),
        ),
    ];
    for (written, expected) in accepted {
        assert_eq!(
            Number::parse_written(&written),
            Some(expected),
            "should accept: {written}"
        );
    }

    let refused = [
        json!("1,5"),
        json!("1e3"),
        json!(" 7 "),
        json!("1.0"),
        json!(""),
        json!("-"),
        json!("+7"),
        json!("ten"),
        json!(true),
        json!(null),
        json!([1]),
    ];
    for written in refused {
        assert_eq!(
            Number::parse_written(&written),
            None,
            "should refuse: {written}"
        );
    }
}

#[test]
fn every_number_comparison_reads_as_written() {
    let cases = [
        (">", 11, true),
        (">", 10, false),
        (">=", 10, true),
        (">=", 9, false),
        ("<", 9, true),
        ("<", 10, false),
        ("<=", 10, true),
        ("<=", 11, false),
        ("==", 10, true),
        ("==", 11, false),
        ("!=", 11, true),
        ("!=", 10, false),
    ];

    for (op, value, expected) in cases {
        let rows = result(&["total"], &[&[json!(value)]]);

        assert_eq!(
            evaluate(&rows, &one_total(op, &json!(10)), &[], &[], None, now()),
            seen(expected, json!(value)),
            "should decide: {value} {op} 10"
        );
    }
}

#[test]
fn a_notification_is_owed_on_the_first_breach_and_again_only_after_a_valid_clear() {
    let breach = seen(true, json!(1));
    let clear = seen(false, json!(0));
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
    let clear = seen(false, json!(0));

    assert_eq!(last_valid_breached(Some(true), &unknown), Some(true));
    assert_eq!(last_valid_breached(None, &unknown), None);
    assert_eq!(last_valid_breached(Some(true), &clear), Some(false));
}

#[test]
fn a_breach_that_never_clears_is_notified_once_across_many_checks() {
    let breach = seen(true, json!(5));
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
        "name": "Too many open PRs",
        "metric": "prs-open",
        "condition": { "rows": "one", "where": { "column": "total", "op": ">", "value": 10 } },
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
            "blank name",
            draft(|body| body["name"] = json!("   ")),
            RuleError::Name,
        ),
        (
            "name too long",
            draft(|body| body["name"] = json!("x".repeat(201))),
            RuleError::Name,
        ),
        (
            "metric name",
            draft(|body| body["metric"] = json!("not a name!")),
            RuleError::Metric,
        ),
        (
            "a column the metric does not answer",
            draft(|body| body["condition"]["where"]["column"] = json!("to-tal")),
            RuleError::Condition(ConditionError {
                path: "where.column".to_owned(),
                problem: Problem::NoSuchColumn("to-tal".to_owned()),
            }),
        ),
        (
            "threshold text that is not an integer",
            draft(|body| body["condition"]["where"]["value"] = json!("1e3")),
            RuleError::Condition(ConditionError {
                path: "where.value".to_owned(),
                problem: Problem::NumberValue,
            }),
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
            RuleSpec::parse(&draft, limits, &destinations(), &metric_columns()).err(),
            Some(expected),
            "should refuse: {case}"
        );
    }

    let ok = RuleSpec::parse(&draft(|_| {}), limits, &destinations(), &metric_columns());
    assert!(ok.is_ok(), "a whole draft is a rule: {ok:?}");
}

#[test]
fn context_columns_name_up_to_five_columns_the_metric_answers() {
    use super::rule::ContextColumnsProblem;

    let limits = Limits::default();
    let columns = Columns {
        kinds: vec![
            ("team".to_owned(), ColumnKind::Text),
            ("total".to_owned(), ColumnKind::Number),
            ("assignee".to_owned(), ColumnKind::Text),
            ("task_id".to_owned(), ColumnKind::Number),
        ],
        groups: vec!["team".to_owned()],
        imposed_limit: None,
    };
    let refused = [
        (
            "six columns",
            json!([
                "total", "assignee", "task_id", "total", "assignee", "task_id"
            ]),
            ContextColumnsProblem::TooMany,
        ),
        (
            "a name that is not a column name",
            json!(["to-tal"]),
            ContextColumnsProblem::Name,
        ),
        (
            "a column named twice",
            json!(["assignee", "task_id", "assignee"]),
            ContextColumnsProblem::Duplicate("assignee".to_owned()),
        ),
        (
            "a column the metric does not answer",
            json!(["assignee", "nobody"]),
            ContextColumnsProblem::NoSuchColumn("nobody".to_owned()),
        ),
        (
            "a group-by column, which the witness shows anyway",
            json!(["assignee", "team"]),
            ContextColumnsProblem::GroupBy("team".to_owned()),
        ),
    ];

    for (case, context_columns, problem) in refused {
        let draft = draft(|body| body["context_columns"] = context_columns);
        assert_eq!(
            RuleSpec::parse(&draft, limits, &destinations(), &columns).err(),
            Some(RuleError::ContextColumns(problem)),
            "should refuse: {case}"
        );
    }

    let named = draft(|body| body["context_columns"] = json!(["task_id", "assignee", "total"]));
    let spec = RuleSpec::parse(&named, limits, &destinations(), &columns)
        .unwrap_or_else(|error| panic!("named columns are a rule: {error}"));
    assert_eq!(
        spec.context_columns,
        ["task_id", "assignee", "total"],
        "in the order written, a condition column among them"
    );

    let unnamed = RuleSpec::parse(&draft(|_| {}), limits, &destinations(), &columns)
        .unwrap_or_else(|error| panic!("a draft without them is a rule: {error}"));
    assert!(unnamed.context_columns.is_empty());
}

#[test]
fn a_range_is_read_as_a_run_would_read_it() {
    let limits = Limits::default();

    let bad = draft(|body| body["range"] = json!("yesterday"));
    assert!(matches!(
        RuleSpec::parse(&bad, limits, &destinations(), &metric_columns()),
        Err(RuleError::Range(_))
    ));

    let good = draft(|body| body["range"] = json!("P7D"));
    let spec = RuleSpec::parse(&good, limits, &destinations(), &metric_columns())
        .unwrap_or_else(|error| panic!("P7D is a range: {error}"));
    let window = spec
        .window()
        .unwrap_or_else(|error| panic!("the window parses: {error}"));
    assert!(window.is_ranged());
}

#[test]
fn a_draft_names_only_the_fields_a_rule_has() {
    let extra = json!({
        "name": "n", "metric": "m",
        "condition": { "rows": "one", "where": { "column": "total", "op": ">", "value": 1 } },
        "interval_secs": 60, "destination": "ops", "colour": "red"
    });

    assert!(serde_json::from_value::<RuleDraft>(extra).is_err());
}

/// One send through a deliverer over the in-memory store, with a provider
/// played in-process.
mod deliveries {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use async_trait::async_trait;
    use chrono::Utc;
    use serde_json::json;

    use insight_log_context::test_support::capture_output;

    use super::super::delivery::{
        Attempted, Delivered, Deliverer, DeliveryJob, Message, Provider, Receipt, SendError,
    };
    use super::super::rule::{
        AlertRule, AlertStore, Limits, Notification, NotificationStatus, Recorded, Recording,
        RuleSpec, Write,
    };
    use crate::store::alerts::memory::MemoryAlerts;

    fn block_on<T>(future: impl Future<Output = T>) -> T {
        futures::executor::block_on(future)
    }

    /// A rule whose first check breached: the notification it owes, pending.
    async fn owed(store: &MemoryAlerts) -> (AlertRule, Notification) {
        let spec = RuleSpec::parse(
            &super::draft(|body| body["condition"]["where"]["value"] = json!(10)),
            Limits::default(),
            &super::destinations(),
            &super::metric_columns(),
        )
        .unwrap_or_else(|error| panic!("the draft is a rule: {error}"));
        let write = Write {
            spec,
            enabled: true,
            actor: None,
        };
        let rule = store
            .create(write, u64::MAX)
            .await
            .unwrap_or_else(|error| panic!("the rule is stored: {error}"));

        let recorded = store
            .record(Recording {
                rule_id: rule.id,
                revision: rule.revision,
                outcome: super::seen(true, json!(12)),
                evaluated_at: Utc::now(),
            })
            .await
            .unwrap_or_else(|error| panic!("the breach is recorded: {error}"));
        let Recorded::Accepted(accepted) = recorded else {
            panic!("the breach is accepted: {recorded:?}");
        };
        let notification = accepted
            .notification
            .unwrap_or_else(|| panic!("the breach owes a notification"));

        (accepted.rule, notification)
    }

    fn row(store: &MemoryAlerts, id: uuid::Uuid) -> Notification {
        block_on(store.notification(id))
            .unwrap_or_else(|error| panic!("the row reads: {error}"))
            .unwrap_or_else(|| panic!("the row is there"))
    }

    fn at(provider: Arc<dyn Provider>) -> BTreeMap<String, Arc<dyn Provider>> {
        BTreeMap::from([("ops".to_owned(), provider)])
    }

    #[derive(Debug)]
    struct Unconfirming;

    #[async_trait]
    impl Provider for Unconfirming {
        async fn send(&self, _: &Message) -> Result<Receipt, SendError> {
            Err(SendError::Unconfirmed("timed out".to_owned()))
        }
    }

    #[test]
    fn the_attempt_cap_counts_the_notification_not_the_job() {
        let store = MemoryAlerts::new();
        let (_, notification) = block_on(owed(&store));
        for _ in 0..4 {
            block_on(store.record_attempt(notification.id, &Attempted::Retry("late".to_owned())))
                .unwrap_or_else(|error| panic!("an attempt is recorded: {error}"));
        }
        let providers = at(Arc::new(Unconfirming));
        let deliverer = Deliverer::new(&store, &providers);
        let job = DeliveryJob {
            notification_id: notification.id,
        };

        let fifth = block_on(deliverer.deliver(job, 1, 5))
            .unwrap_or_else(|error| panic!("the send is recorded: {error}"));

        assert!(
            matches!(fifth, Delivered::Attempted(Attempted::Failed(_))),
            "a fresh job on the fifth attempt of the row is the last one: {fifth:?}"
        );
        let settled = row(&store, notification.id);
        assert_eq!(
            (settled.status, settled.attempts),
            (NotificationStatus::Failed, 5)
        );
    }

    #[test]
    fn a_job_further_along_than_its_row_keeps_its_own_count() {
        let store = MemoryAlerts::new();
        let (_, notification) = block_on(owed(&store));
        let providers = at(Arc::new(Unconfirming));
        let deliverer = Deliverer::new(&store, &providers);
        let job = DeliveryJob {
            notification_id: notification.id,
        };

        let early = block_on(deliverer.deliver(job, 2, 5))
            .unwrap_or_else(|error| panic!("the send is recorded: {error}"));
        assert!(
            matches!(early, Delivered::Attempted(Attempted::Retry(_))),
            "{early:?}"
        );

        let last = block_on(deliverer.deliver(job, 5, 5))
            .unwrap_or_else(|error| panic!("the send is recorded: {error}"));
        assert!(
            matches!(last, Delivered::Attempted(Attempted::Failed(_))),
            "{last:?}"
        );
        assert_eq!(row(&store, notification.id).attempts, 2);
    }

    /// A provider that, while the message is in flight, has the rule turned
    /// off — and confirms the message anyway.
    #[derive(Debug)]
    struct Withdrawing {
        store: Arc<MemoryAlerts>,
        rule: AlertRule,
    }

    #[async_trait]
    impl Provider for Withdrawing {
        async fn send(&self, _: &Message) -> Result<Receipt, SendError> {
            self.store
                .set_enabled(self.rule.id, self.rule.revision, false)
                .await
                .unwrap_or_else(|error| panic!("the rule is disabled: {error}"));

            Ok(Receipt("m-late".to_owned()))
        }
    }

    #[test]
    fn a_send_confirmed_while_withdrawn_is_skipped_and_its_receipt_is_logged() {
        let store = Arc::new(MemoryAlerts::new());
        let (rule, notification) = block_on(owed(&store));
        let providers = at(Arc::new(Withdrawing {
            store: Arc::clone(&store),
            rule,
        }));
        let deliverer = Deliverer::new(store.as_ref(), &providers);
        let job = DeliveryJob {
            notification_id: notification.id,
        };

        let mut outcome = None;
        let output = capture_output(|| {
            outcome = Some(block_on(deliverer.deliver(job, 1, 5)));
        });

        assert!(
            matches!(outcome, Some(Ok(Delivered::Skipped))),
            "{outcome:?}"
        );
        let withdrawn = row(&store, notification.id);
        assert_eq!(
            (
                withdrawn.status,
                withdrawn.attempts,
                withdrawn.provider_receipt
            ),
            (NotificationStatus::Cancelled, 0, None)
        );
        assert!(
            output.contains(&notification.id.to_string()) && output.contains("m-late"),
            "the log is where the receipt went: {output}"
        );
    }
}

#[test]
fn a_threshold_is_worded_exactly_as_javascript_writes_it() {
    let cases = [
        (Number::Float(123_456.5), "123456.5"),
        (Number::Float(1e-20), "1e-20"),
        (Number::Float(1e21), "1e+21"),
        (Number::Float(1e20), "100000000000000000000"),
        (Number::Float(0.000_001), "0.000001"),
        (Number::Float(1e-7), "1e-7"),
        (Number::Float(-2.5), "-2.5"),
        (Number::Float(10.0), "10"),
        (Number::Float(0.1 + 0.2), "0.30000000000000004"),
        (Number::Float(1.234_5e300), "1.2345e+300"),
        (Number::Float(-0.0), "0"),
        (Number::Int(-9_007_199_254_740_993), "-9007199254740993"),
    ];

    for (number, expected) in cases {
        assert_eq!(number.exact(), expected, "should write {number:?}");
    }
}
