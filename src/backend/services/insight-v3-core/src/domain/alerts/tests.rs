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

    let at_the_bound = result(&["total"], &[&[json!(9_007_199_254_740_992_u64)]]);
    assert_eq!(
        classify(&at_the_bound, "total", &above(&json!(1.5))),
        Outcome::Valid {
            value: Number::Int(9_007_199_254_740_992),
            breached: true,
        },
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
        "name": "Too many open PRs",
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
            "column",
            draft(|body| body["column"] = json!("to-tal")),
            RuleError::Column,
        ),
        (
            "threshold text that is not an integer",
            draft(|body| body["threshold"] = json!("1e3")),
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
        "name": "n", "metric": "m", "column": "c", "operator": ">", "threshold": 1,
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
    use super::super::{Number, Outcome};
    use crate::store::alerts::memory::MemoryAlerts;

    fn block_on<T>(future: impl Future<Output = T>) -> T {
        futures::executor::block_on(future)
    }

    /// A rule whose first check breached: the notification it owes, pending.
    async fn owed(store: &MemoryAlerts) -> (AlertRule, Notification) {
        let spec = RuleSpec::parse(
            &super::draft(|body| body["threshold"] = json!(10)),
            Limits::default(),
            &super::destinations(),
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
                outcome: Outcome::Valid {
                    value: Number::Int(12),
                    breached: true,
                },
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
