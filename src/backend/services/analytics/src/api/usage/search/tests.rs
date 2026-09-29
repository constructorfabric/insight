use toolkit_canonical_errors::Problem;

use super::*;

type R = Result<(), Box<dyn std::error::Error>>;

#[test]
fn a_blank_search_narrows_nothing() -> R {
    for raw in [None, Some(""), Some("   ")] {
        assert!(
            PersonSearch::parse(raw)?.is_none(),
            "should narrow nothing: {raw:?}"
        );
    }
    Ok(())
}

#[test]
fn a_search_is_matched_without_its_surrounding_whitespace() -> R {
    let search = PersonSearch::parse(Some("  ada  "))?;

    assert_eq!(search.as_ref().map(PersonSearch::as_str), Some("ada"));
    Ok(())
}

#[test]
fn a_search_past_the_budget_is_refused_naming_the_field() -> R {
    assert!(PersonSearch::parse(Some(&"x".repeat(MAX_SEARCH_BYTES)))?.is_some());

    let Err(error) = PersonSearch::parse(Some(&"x".repeat(MAX_SEARCH_BYTES + 1))) else {
        panic!("a search past the budget is refused")
    };
    let refused = serde_json::to_value(Problem::from(error))?;

    assert_eq!(refused["status"], 400);
    assert_eq!(refused["context"]["field_violations"][0]["field"], "search");
    Ok(())
}
