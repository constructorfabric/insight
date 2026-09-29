use super::*;

#[test]
fn the_actions_breakdown_leaves_out_what_nobody_did() {
    assert!(actions_sql("1").contains("NOT IN ('page_view', 'session_start')"));
}

#[test]
fn every_caller_value_in_a_read_is_a_placeholder() {
    let visitors = "V";
    assert_eq!(WINDOW.matches('?').count(), 3);
    for sql in [by_page_sql(visitors), actions_sql(visitors)] {
        assert_eq!(
            sql.matches('?').count(),
            3,
            "a read that does not bind exactly the window has interpolated a value: {sql}"
        );
    }
    assert_eq!(
        people_sql().matches('?').count(),
        4,
        "people_query binds the tenant a second time for the identity join"
    );
}

#[test]
fn a_visitor_is_named_from_the_mirrored_identity_rows() {
    let sql = people_sql();
    assert!(sql.contains("identity.identity_persons"), "{sql}");
    assert!(sql.contains("display_name"), "{sql}");
}

#[test]
fn a_visitor_without_a_name_is_still_named_by_its_account_handle() {
    let sql = people_sql();

    assert!(sql.contains("'username'"), "{sql}");
    assert!(sql.contains("AS username"), "{sql}");
}
