use super::*;

fn owned(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn spelled(names: &[TagName]) -> Vec<&str> {
    names.iter().map(TagName::as_str).collect()
}

#[test]
fn a_name_is_stored_without_the_spaces_around_it() {
    let parsed = TagName::parse("  Delivery  ")
        .unwrap_or_else(|error| panic!("a padded name must parse: {error}"));

    assert_eq!(parsed.as_str(), "Delivery");
}

#[test]
fn a_name_of_nothing_but_spaces_is_refused() {
    for blank in ["", "   ", "\t"] {
        assert!(
            matches!(TagName::parse(blank), Err(TagError::Name)),
            "{blank:?}"
        );
    }
}

#[test]
fn a_name_may_run_to_thirty_two_characters_and_no_further() {
    assert!(TagName::parse(&"é".repeat(32)).is_ok());
    assert!(matches!(
        TagName::parse(&"é".repeat(33)),
        Err(TagError::Name)
    ));
}

#[test]
fn names_equal_but_for_case_collapse_to_the_first_spelling() {
    let set = TagSet::parse(&owned(&["Delivery", "delivery", " DELIVERY ", "Hiring"]))
        .unwrap_or_else(|error| panic!("{error}"));

    assert_eq!(spelled(set.names()), ["Delivery", "Hiring"]);
}

#[test]
fn a_dashboard_carries_ten_tags_and_no_more() {
    let ten: Vec<String> = (0..10).map(|index| format!("t{index}")).collect();
    let eleven: Vec<String> = (0..11).map(|index| format!("t{index}")).collect();

    assert_eq!(
        TagSet::parse(&ten)
            .unwrap_or_else(|error| panic!("{error}"))
            .names()
            .len(),
        10
    );
    assert!(matches!(
        TagSet::parse(&eleven),
        Err(TagError::TooManyOnDashboard)
    ));
}

#[test]
fn eleven_spellings_of_ten_tags_are_ten_tags() {
    let mut values: Vec<String> = (0..10).map(|index| format!("t{index}")).collect();
    values.push("T0".to_owned());

    assert!(TagSet::parse(&values).is_ok());
}

#[test]
fn a_set_holding_a_blank_name_is_refused_whole() {
    assert!(matches!(
        TagSet::parse(&owned(&["Delivery", " "])),
        Err(TagError::Name)
    ));
}

#[test]
fn an_empty_set_clears() {
    let set = TagSet::parse(&[]).unwrap_or_else(|error| panic!("{error}"));

    assert!(set.names().is_empty());
}

#[test]
fn no_tag_named_is_no_filter_at_all() {
    for kind in DefinitionKind::ALL {
        assert!(
            matches!(TagFilter::parse(kind, &[]), Ok(None)),
            "{}",
            kind.plural()
        );
    }
}

#[test]
fn a_filter_reads_each_tag_named() {
    let filter = TagFilter::parse(DefinitionKind::Dashboard, &owned(&["Delivery", " hiring "]))
        .unwrap_or_else(|error| panic!("{error}"));

    let Some(filter) = filter else {
        panic!("two tags named are a filter");
    };
    assert_eq!(spelled(filter.names()), ["Delivery", "hiring"]);
}

#[test]
fn only_dashboards_are_filtered_by_tag() {
    for kind in [DefinitionKind::Metric, DefinitionKind::Widget] {
        let refused = TagFilter::parse(kind, &owned(&["Delivery"]));

        assert!(
            matches!(refused, Err(TagError::NotTagged(plural)) if plural == kind.plural()),
            "{refused:?}"
        );
    }
}

#[test]
fn a_filter_by_a_blank_name_is_refused() {
    assert!(matches!(
        TagFilter::parse(DefinitionKind::Dashboard, &owned(&[""])),
        Err(TagError::Name)
    ));
}

#[test]
fn a_filter_names_no_more_tags_than_can_exist() {
    let every: Vec<String> = (0..MAX_TAGS).map(|index| format!("t{index}")).collect();
    let mut beyond = every.clone();
    beyond.push("one more".to_owned());

    assert!(TagFilter::parse(DefinitionKind::Dashboard, &every).is_ok());
    assert!(matches!(
        TagFilter::parse(DefinitionKind::Dashboard, &beyond),
        Err(TagError::FilterTooWide)
    ));
}

#[test]
fn only_a_store_failure_is_not_the_caller_s_to_act_on() {
    let refusals = [
        TagError::Name,
        TagError::TooManyOnDashboard,
        TagError::FilterTooWide,
        TagError::NotTagged("metrics"),
        TagError::DashboardNotFound("delivery".to_owned()),
        TagError::TooMany,
    ];
    let down = TagError::Store(DefinitionStoreError::Database(sea_orm::DbErr::Custom(
        "down".to_owned(),
    )));

    for refusal in &refusals {
        assert!(refusal.is_about_the_caller(), "{refusal}");
    }
    assert!(!down.is_about_the_caller());
}
