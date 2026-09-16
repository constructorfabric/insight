use chrono::TimeZone as _;

use super::*;

fn at(second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 16, 12, 0, second)
        .single()
        .unwrap_or_else(|| panic!("the fixture time exists"))
}

fn name() -> DefinitionName {
    DefinitionName::parse("commits").unwrap_or_else(|error| panic!("the name parses: {error}"))
}

fn row(state: DatasetState, held: Option<Held>) -> Dataset {
    Dataset {
        name: name(),
        declaration: serde_json::json!({ "fields": [] }),
        state,
        physical_table: None,
        held,
    }
}

fn held(operation: Operation, until: u32) -> Held {
    Held {
        operation,
        token: OperationToken::mint(),
        until: at(until),
    }
}

#[test]
fn a_name_nobody_holds_is_claimed_by_the_insert_itself() {
    assert_eq!(taking(None, Operation::Create, at(0)), Taking::Claim);
}

#[test]
fn a_dataset_that_was_never_there_has_nothing_to_remove() {
    assert_eq!(taking(None, Operation::Remove, at(0)), Taking::Gone);
}

#[test]
fn an_operation_whose_lease_still_holds_refuses_a_second_attempt() {
    let cases = [
        (Operation::Create, Operation::Create),
        (Operation::Create, Operation::Remove),
        (Operation::Remove, Operation::Remove),
    ];

    for (under_way, asked) in cases {
        let dataset = row(DatasetState::Claimed, Some(held(under_way, 30)));

        assert_eq!(
            taking(Some(&dataset), asked, at(0)),
            Taking::Refuse(Refused::Busy(under_way)),
            "should be refused while a {under_way} holds it"
        );
    }
}

#[test]
fn an_operation_whose_lease_has_lapsed_may_be_taken_over() {
    let dataset = row(DatasetState::Claimed, Some(held(Operation::Create, 10)));

    assert_eq!(
        taking(Some(&dataset), Operation::Create, at(30)),
        Taking::Take(DatasetState::Claimed)
    );
}

#[test]
fn a_name_under_removal_is_not_free_to_create_again() {
    let dataset = row(DatasetState::Removing, None);

    assert_eq!(
        taking(Some(&dataset), Operation::Create, at(0)),
        Taking::Refuse(Refused::Removing)
    );
}

#[test]
fn a_removal_that_was_abandoned_may_be_repeated() {
    let dataset = row(DatasetState::Removing, Some(held(Operation::Remove, 10)));

    assert_eq!(
        taking(Some(&dataset), Operation::Remove, at(30)),
        Taking::Take(DatasetState::Removing)
    );
}

#[test]
fn an_abandoned_create_may_be_removed() {
    let dataset = row(DatasetState::Claimed, None);

    assert_eq!(
        taking(Some(&dataset), Operation::Remove, at(0)),
        Taking::Take(DatasetState::Removing)
    );
}

#[test]
fn a_table_is_named_for_the_attempt_and_never_for_the_dataset() {
    let token = OperationToken::mint();

    assert!(token.table().starts_with("ds_"), "{}", token.table());
    assert!(!token.table().contains("commits"));
    assert_ne!(OperationToken::mint().table(), token.table());
}

#[test]
fn a_lease_lapses_a_bounded_time_after_it_is_taken() {
    assert_eq!(lease_until(at(0)), at(0) + TimeDelta::seconds(LEASE_SECS));
}
