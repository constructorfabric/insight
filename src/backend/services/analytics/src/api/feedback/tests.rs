use std::collections::HashMap;

use toolkit_canonical_errors::Problem;

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

fn stored(person_id: Uuid, message: &str) -> feedback::Model {
    feedback::Model {
        id: Uuid::now_v7(),
        insight_tenant_id: Uuid::now_v7(),
        person_id,
        message: message.to_owned(),
        path: "/portal/overview".to_owned(),
        created_at: Utc::now(),
    }
}

fn problem(error: CanonicalError) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::to_value(Problem::from(error))
}

#[test]
fn a_sender_the_identity_rows_name_is_named_in_the_listing() {
    let person = Uuid::now_v7();
    let mut names = HashMap::new();
    names.insert(person, PersonName::named("Alice Example", "alice"));

    let row = entry(stored(person, "the chart is empty"), &names);

    assert_eq!(row.display_name, "Alice Example");
    assert_eq!(row.username, "alice");
    assert_eq!(row.person_id, person.to_string());
    assert_eq!(row.message, "the chart is empty");
}

#[test]
fn a_sender_the_mirror_does_not_carry_is_still_listed() {
    let row = entry(stored(Uuid::now_v7(), "a note"), &HashMap::new());

    assert_eq!(row.display_name, "");
    assert_eq!(row.username, "");
    assert!(
        !row.person_id.is_empty(),
        "the id is what names them instead"
    );
}

#[test]
fn the_listing_timestamp_is_a_whole_second_in_utc() {
    let row = entry(stored(Uuid::now_v7(), "a note"), &HashMap::new());

    assert_eq!(row.ts.len(), "2026-08-22 05:31:59".len());
    assert!(row.ts.contains(':'), "{}", row.ts);
}

#[test]
fn the_mirror_is_asked_once_per_person_not_once_per_row() {
    let loud = Uuid::now_v7();
    let quiet = Uuid::now_v7();
    let rows = vec![
        stored(loud, "first"),
        stored(loud, "second"),
        stored(quiet, "third"),
        stored(loud, "fourth"),
    ];

    let asked = senders(&rows);

    assert_eq!(asked.len(), 2, "{asked:?}");
    assert!(asked.contains(&loud) && asked.contains(&quiet), "{asked:?}");
}

#[test]
fn nobody_to_name_asks_nothing() {
    assert!(senders(&[]).is_empty());
}

#[test]
fn a_non_admin_is_refused_in_the_feedback_namespace() -> Result<(), serde_json::Error> {
    let refusal = problem(admin_only())?;

    assert_eq!(refusal["status"], 403);
    assert_eq!(
        refusal["context"]["resource_type"],
        "gts.cf.insight.analytics_api.feedback.v1~"
    );
    assert_eq!(refusal["context"]["reason"], ADMIN_ONLY);
    Ok(())
}

#[test]
fn a_refused_window_names_the_field_and_the_rule() -> Result<(), serde_json::Error> {
    let refusal = problem(refused_window(WindowError::TooWide))?;

    assert_eq!(refusal["status"], 400);
    assert_eq!(refusal["context"]["field_violations"][0]["field"], "since");
    Ok(())
}

#[test]
fn a_failed_read_says_nothing_about_the_database() -> Result<(), serde_json::Error> {
    let refusal = problem(read_error(sea_orm::DbErr::Custom(
        "connection to db-7 refused".to_owned(),
    )))?;

    assert_eq!(refusal["status"], 500);
    assert!(
        !refusal.to_string().contains("db-7"),
        "the wire error must not carry the internal detail: {refusal}"
    );
    Ok(())
}
