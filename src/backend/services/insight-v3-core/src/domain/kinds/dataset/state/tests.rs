use super::*;

#[test]
fn every_state_survives_the_round_trip_through_its_row() {
    for state in [
        DatasetState::Claimed,
        DatasetState::Ready,
        DatasetState::Removing,
    ] {
        assert_eq!(
            DatasetState::parse(state.as_str()),
            Some(state),
            "should round-trip: {state}"
        );
    }
}

#[test]
fn every_operation_survives_the_round_trip_through_its_row() {
    for operation in [Operation::Create, Operation::Remove] {
        assert_eq!(
            Operation::parse(operation.as_str()),
            Some(operation),
            "should round-trip: {operation}"
        );
    }
}

#[test]
fn a_word_this_service_never_wrote_is_no_state_at_all() {
    assert_eq!(DatasetState::parse("absent"), None);
    assert_eq!(DatasetState::parse(""), None);
    assert_eq!(Operation::parse("replace"), None);
}
