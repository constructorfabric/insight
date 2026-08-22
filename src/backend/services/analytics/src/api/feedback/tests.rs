use super::*;

fn request(message: &str) -> FeedbackRequest {
    FeedbackRequest {
        message: message.to_owned(),
        path: "/portal/overview".to_owned(),
    }
}

fn row_of(req: &FeedbackRequest) -> Option<feedback::ActiveModel> {
    to_row(req, Uuid::now_v7(), Uuid::now_v7(), Utc::now()).ok()
}

#[test]
fn the_session_decides_identity_not_the_payload() {
    let tenant = Uuid::now_v7();
    let person = Uuid::now_v7();

    let Some(row) = to_row(&request("the chart is empty"), tenant, person, Utc::now()).ok() else {
        panic!("a filled message is accepted")
    };

    assert_eq!(row.insight_tenant_id.try_as_ref(), Some(&tenant));
    assert_eq!(row.person_id.try_as_ref(), Some(&person));
}

#[test]
fn a_message_of_whitespace_is_refused_rather_than_stored() {
    for message in ["", "   ", "\n\t "] {
        assert!(
            row_of(&request(message)).is_none(),
            "should reject: {message:?}"
        );
    }
}

#[test]
fn a_stored_message_carries_no_surrounding_whitespace() {
    let row = row_of(&request("  add a dark mode  "));

    assert_eq!(
        row.as_ref().and_then(|r| r.message.try_as_ref()),
        Some(&"add a dark mode".to_owned())
    );
}

#[test]
fn an_oversized_message_is_clipped_to_the_column_budget() {
    let budget = feedback_schema::max_message();
    let long = "x".repeat(budget * 2);

    let row = row_of(&request(&long));

    assert_eq!(
        row.as_ref()
            .and_then(|r| r.message.try_as_ref())
            .map(|m| m.chars().count()),
        Some(budget)
    );
}

#[test]
fn the_screen_the_sender_was_on_is_stored_with_what_they_wrote() {
    let row = row_of(&request("this is confusing"));

    assert_eq!(
        row.as_ref().and_then(|r| r.path.try_as_ref()),
        Some(&"/portal/overview".to_owned())
    );
}

#[test]
fn the_last_day_of_the_window_is_read_whole() {
    let Some(day) = NaiveDate::from_ymd_opt(2026, 8, 22) else {
        panic!("a real date")
    };

    assert_eq!(
        day_after(day).format("%Y-%m-%d %H:%M:%S").to_string(),
        "2026-08-23 00:00:00"
    );
    assert_eq!(
        day_start(day).format("%Y-%m-%d %H:%M:%S").to_string(),
        "2026-08-22 00:00:00"
    );
}
