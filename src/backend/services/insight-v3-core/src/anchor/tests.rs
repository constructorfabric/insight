use super::*;

fn parsed(body: &str) -> Anchor {
    Anchor::parse(body.as_bytes()).unwrap_or_else(|error| panic!("the answer parses: {error}"))
}

#[test]
fn a_count_arrives_whether_it_is_written_as_text_or_a_number() {
    assert_eq!(
        parsed(r#"{"meta":[],"data":[{"undated":"3"}]}"#).undated(),
        3
    );
    assert_eq!(parsed(r#"{"meta":[],"data":[{"undated":7}]}"#).undated(), 7);
}

#[test]
fn a_source_whose_clocks_are_all_null_counts_every_row_as_undated() {
    let anchor = parsed(r#"{"meta":[],"data":[{"undated":"7"}]}"#);

    assert_eq!(anchor.undated(), 7);
}

#[test]
fn an_empty_source_answers_no_row_at_all() {
    let anchor = parsed(r#"{"meta":[],"data":[]}"#);

    assert_eq!(anchor, Anchor::default());
}
