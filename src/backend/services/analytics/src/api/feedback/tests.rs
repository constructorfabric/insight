use super::*;

fn request(category: FeedbackCategory, message: &str) -> FeedbackRequest {
    FeedbackRequest {
        category,
        message: message.to_owned(),
        path: "/portal/overview".to_owned(),
        app_name: "insight-front".to_owned(),
        app_version: "0.0.1".to_owned(),
    }
}

fn row_of(req: &FeedbackRequest) -> Option<FeedbackRow> {
    to_row(req, Uuid::now_v7(), Uuid::now_v7(), Utc::now()).ok()
}

#[test]
fn the_session_decides_identity_not_the_payload() {
    let tenant = Uuid::now_v7();
    let person = Uuid::now_v7();

    let row = to_row(
        &request(FeedbackCategory::Bug, "the chart is empty"),
        tenant,
        person,
        Utc::now(),
    )
    .ok();

    assert_eq!(row.as_ref().map(|r| r.tenant_id), Some(tenant));
    assert_eq!(row.as_ref().map(|r| r.person_id), Some(person));
}

#[test]
fn a_message_of_whitespace_is_refused_rather_than_stored() {
    for message in ["", "   ", "\n\t "] {
        assert!(
            row_of(&request(FeedbackCategory::Other, message)).is_none(),
            "should reject: {message:?}"
        );
    }
}

#[test]
fn a_stored_message_carries_no_surrounding_whitespace() {
    let row = row_of(&request(FeedbackCategory::Idea, "  add a dark mode  "));

    assert_eq!(row.map(|r| r.message), Some("add a dark mode".to_owned()));
}

#[test]
fn an_oversized_message_is_clipped_to_the_column_budget() {
    let long = "x".repeat(MAX_MESSAGE * 2);

    let row = row_of(&request(FeedbackCategory::Other, &long));

    assert_eq!(row.map(|r| r.message.chars().count()), Some(MAX_MESSAGE));
}

#[test]
fn a_category_is_stored_as_the_wire_name_the_dialog_sends() {
    let cases = [
        (FeedbackCategory::Bug, "bug"),
        (FeedbackCategory::Idea, "idea"),
        (FeedbackCategory::Confusing, "confusing"),
        (FeedbackCategory::Other, "other"),
    ];

    for (category, expected) in cases {
        let row = row_of(&request(category, "a note"));

        assert_eq!(
            row.map(|r| r.category),
            Some(expected.to_owned()),
            "should store: {expected}"
        );
    }
}

#[test]
fn a_category_outside_the_closed_set_never_parses() {
    let unknown = serde_json::json!({ "category": "rant", "message": "a note" });

    assert!(
        serde_json::from_value::<FeedbackRequest>(unknown).is_err(),
        "an unknown category is a 400, not a silent 'other'"
    );
}

#[test]
fn every_caller_value_in_the_listing_is_a_placeholder() {
    let sql = list_sql();

    assert_eq!(
        sql.matches('?').count(),
        4,
        "the listing binds the window and the tenant again for the identity join: {sql}"
    );
}

#[test]
fn a_sender_is_named_from_the_mirrored_identity_rows() {
    let sql = list_sql();

    assert!(sql.contains("identity.identity_persons"), "{sql}");
    assert!(sql.contains("AS display_name"), "{sql}");
    assert!(sql.contains("AS username"), "{sql}");
}

#[test]
fn the_newest_feedback_is_read_first() {
    let sql = list_sql();

    assert!(sql.contains("ORDER BY f.ts DESC"), "{sql}");
}

#[test]
fn a_malformed_day_is_refused_rather_than_queried() {
    assert!(
        date_window::parse_window(Some("2026-99-99"), None, violation).is_err(),
        "a date that cannot exist is a 400"
    );
}
