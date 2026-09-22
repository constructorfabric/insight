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
fn a_table_carries_the_dataset_a_reader_knows_and_the_attempt_that_made_it() {
    let token = OperationToken::mint();
    let table = token.table(&name());

    assert!(table.starts_with("ds_commits_"), "{table}");
    assert_ne!(
        OperationToken::mint().table(&name()),
        table,
        "two attempts at one dataset must never address one table"
    );
}

/// Taking a dataset that stands for a create would demote it out of sight,
/// make a second table and leave its records in the first.
#[test]
fn a_create_never_takes_a_dataset_that_stands() {
    let stands = row(DatasetState::Ready, None);

    assert_eq!(
        taking(Some(&stands), Operation::Create, at(0)),
        Taking::Stands
    );
    assert_eq!(
        taking(Some(&stands), Operation::Remove, at(0)),
        Taking::Take(DatasetState::Removing),
        "a removal does take one that stands"
    );
}

#[test]
fn an_attempt_writes_only_while_the_row_still_records_it() {
    let token = OperationToken::mint();
    let mine = row(
        DatasetState::Claimed,
        Some(Held {
            operation: Operation::Create,
            token: token.clone(),
            until: at(30),
        }),
    );

    assert_eq!(finishing(Some(&mine), &token), Owning::Held);
    assert_eq!(
        finishing(Some(&mine), &OperationToken::mint()),
        Owning::Lost,
        "another attempt holds it now"
    );
    assert_eq!(finishing(None, &token), Owning::Lost, "the row is gone");
    assert_eq!(
        finishing(Some(&row(DatasetState::Ready, None)), &token),
        Owning::Lost,
        "the operation was released"
    );
}

#[test]
fn a_lease_lapses_a_bounded_time_after_it_is_taken() {
    assert_eq!(
        Lease::default().until(at(0)),
        at(0) + TimeDelta::seconds(LEASE_SECS)
    );
}
