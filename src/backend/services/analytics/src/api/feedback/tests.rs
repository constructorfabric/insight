use sea_orm::ActiveValue;

use super::*;

fn request(message: &str) -> FeedbackRequest {
    FeedbackRequest {
        message: message.to_owned(),
        path: "/portal/overview".to_owned(),
        app_name: "insight-front".to_owned(),
        app_version: "0.0.1".to_owned(),
    }
}

fn row_of(req: &FeedbackRequest) -> Option<feedback::ActiveModel> {
    to_row(req, Uuid::now_v7(), Uuid::now_v7(), Utc::now()).ok()
}

fn value<T>(field: ActiveValue<T>) -> Option<T>
where
    T: Clone + Into<sea_orm::Value>,
{
    match field {
        ActiveValue::Set(v) => Some(v),
        _ => None,
    }
}

#[test]
fn the_session_decides_identity_not_the_payload() {
    let tenant = Uuid::now_v7();
    let person = Uuid::now_v7();

    let row = to_row(&request("the chart is empty"), tenant, person, Utc::now()).ok();

    assert_eq!(
        row.clone().and_then(|r| value(r.insight_tenant_id)),
        Some(tenant)
    );
    assert_eq!(row.and_then(|r| value(r.person_id)), Some(person));
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
        row.and_then(|r| value(r.message)),
        Some("add a dark mode".to_owned())
    );
}

#[test]
fn an_oversized_message_is_clipped_to_the_column_budget() {
    let long = "x".repeat(MAX_MESSAGE * 2);

    let row = row_of(&request(&long));

    assert_eq!(
        row.and_then(|r| value(r.message))
            .map(|m| m.chars().count()),
        Some(MAX_MESSAGE)
    );
}

#[test]
fn the_screen_the_sender_was_on_is_stored_with_what_they_wrote() {
    let row = row_of(&request("this is confusing"));

    assert_eq!(
        row.and_then(|r| value(r.path)),
        Some("/portal/overview".to_owned())
    );
}

#[test]
fn the_last_day_of_the_window_is_read_whole() {
    let day = NaiveDate::from_ymd_opt(2026, 8, 22).unwrap_or_default();

    assert_eq!(
        day_end(day).format("%Y-%m-%d %H:%M:%S").to_string(),
        "2026-08-22 23:59:59"
    );
    assert_eq!(
        day_start(day).format("%Y-%m-%d %H:%M:%S").to_string(),
        "2026-08-22 00:00:00"
    );
}

#[test]
fn a_malformed_day_is_refused_rather_than_queried() {
    assert!(
        date_window::parse_window(Some("2026-99-99"), None, violation).is_err(),
        "a date that cannot exist is a 400"
    );
}
