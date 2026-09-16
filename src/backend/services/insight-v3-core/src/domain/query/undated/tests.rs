use super::*;

fn parsed(body: &str) -> UndatedCount {
    UndatedCount::parse(body.as_bytes())
        .unwrap_or_else(|error| panic!("the answer parses: {error}"))
}

#[test]
fn a_count_arrives_whether_it_is_written_as_text_or_a_number() {
    assert_eq!(parsed(r#"{"meta":[],"data":[{"undated":"3"}]}"#).count(), 3);
    assert_eq!(parsed(r#"{"meta":[],"data":[{"undated":7}]}"#).count(), 7);
}

#[test]
fn a_source_whose_clocks_are_all_null_counts_every_row() {
    assert_eq!(parsed(r#"{"meta":[],"data":[{"undated":"7"}]}"#).count(), 7);
}

#[test]
fn an_empty_source_answers_no_row_at_all() {
    assert_eq!(parsed(r#"{"meta":[],"data":[]}"#), UndatedCount::default());
}
