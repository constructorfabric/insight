use super::*;

#[test]
fn a_row_is_read_and_held_by_name_and_never_interpolates_it() {
    for statement in [SELECT_ROW, SELECT_ROW_HELD] {
        assert!(statement.contains("WHERE name = ?"), "{statement}");
        assert!(statement.contains("FROM datasets"), "{statement}");
    }

    assert!(!SELECT_ROW.contains("FOR UPDATE"));
    assert!(SELECT_ROW_HELD.ends_with("FOR UPDATE"));
}

#[test]
fn claiming_a_name_is_an_insert_that_a_second_writer_loses() {
    assert!(
        CLAIM_NAME.starts_with("INSERT INTO datasets"),
        "{CLAIM_NAME}"
    );
    assert!(!CLAIM_NAME.contains("ON DUPLICATE KEY"), "{CLAIM_NAME}");
}

#[test]
fn taking_an_operation_writes_the_state_the_token_and_the_lease_together() {
    for column in [
        "state = ?",
        "operation = ?",
        "operation_token = ?",
        "lease_until = ?",
    ] {
        assert!(TAKE_OPERATION.contains(column), "{column} is not written");
    }
    assert!(
        TAKE_OPERATION.contains("WHERE name = ?"),
        "{TAKE_OPERATION}"
    );
}

#[test]
fn the_lease_clock_is_the_stores_own() {
    assert!(NOW.contains("UTC_TIMESTAMP(6)"), "{NOW}");
}
