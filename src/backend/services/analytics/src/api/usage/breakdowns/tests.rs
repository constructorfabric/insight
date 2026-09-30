use super::super::search::PersonSearch;
use super::super::sort::{ActionsSort, Order, PagesSort, PeopleSort};
use super::*;

type R = Result<(), Box<dyn std::error::Error>>;

#[test]
fn the_actions_breakdown_leaves_out_what_nobody_did() {
    assert!(actions_sql("1", Order::default()).contains("NOT IN ('page_view', 'session_start')"));
}

#[test]
fn every_caller_value_in_a_read_is_a_placeholder() -> R {
    let visitors = "V";
    assert_eq!(WINDOW.matches('?').count(), 3);
    for sql in [
        by_page_sql(visitors, Order::parse(Some("visitors"), Some("asc"))?),
        actions_sql(visitors, Order::parse(Some("people"), Some("asc"))?),
    ] {
        assert_eq!(
            sql.matches('?').count(),
            3,
            "a read that does not bind exactly the window has interpolated a value: {sql}"
        );
    }
    assert_eq!(
        people_sql(Order::parse(Some("last_seen"), Some("asc"))?, None)
            .matches('?')
            .count(),
        4,
        "people_query binds the tenant a second time for the identity join"
    );
    Ok(())
}

#[test]
fn a_visitor_is_named_from_the_mirrored_identity_rows() {
    let sql = people_sql(Order::default(), None);
    assert!(sql.contains("identity.identity_persons"), "{sql}");
    assert!(sql.contains("display_name"), "{sql}");
}

#[test]
fn a_visitor_without_a_name_is_still_named_by_its_account_handle() {
    let sql = people_sql(Order::default(), None);

    assert!(sql.contains("'username'"), "{sql}");
    assert!(sql.contains("AS username"), "{sql}");
}

#[test]
fn the_cap_keeps_the_rows_the_chosen_order_puts_first() -> R {
    let people = people_sql(
        Order::<PeopleSort>::parse(Some("last_seen"), Some("desc"))?,
        None,
    );
    let pages = by_page_sql(
        "V",
        Order::<PagesSort>::parse(Some("visitors"), Some("asc"))?,
    );
    let actions = actions_sql(
        "V",
        Order::<ActionsSort>::parse(Some("people"), Some("desc"))?,
    );

    assert!(
        people.contains("ORDER BY last_ts DESC, person LIMIT 200"),
        "{people}"
    );
    assert!(
        pages.ends_with("ORDER BY visitors ASC, path LIMIT 200"),
        "{pages}"
    );
    assert!(
        actions.ends_with("ORDER BY people DESC, event_name, target LIMIT 200"),
        "{actions}"
    );
    Ok(())
}

#[test]
fn the_identity_join_keeps_the_order_the_cap_chose() -> R {
    let sql = people_sql(
        Order::<PeopleSort>::parse(Some("page_views"), Some("asc"))?,
        None,
    );

    assert!(
        sql.contains("ORDER BY page_views ASC, visits ASC, person LIMIT 200"),
        "{sql}"
    );
    assert!(
        sql.ends_with("ORDER BY u.page_views ASC, u.visits ASC, u.person"),
        "{sql}"
    );
    Ok(())
}

#[test]
fn last_seen_orders_by_the_instant_and_renders_it_as_text() {
    let sql = people_sql(Order::default(), None);

    assert!(sql.contains("max(ts) AS last_ts"), "{sql}");
    assert!(sql.contains("toString(last_ts) AS last_seen"), "{sql}");
}

fn searching(raw: &str) -> Result<PersonSearch, Box<dyn std::error::Error>> {
    PersonSearch::parse(Some(raw))?.ok_or_else(|| "a real search".into())
}

#[test]
fn a_search_matches_a_visitor_by_name_or_by_handle() -> R {
    let sql = people_sql(Order::default(), Some(&searching("ada")?));

    assert!(
        sql.contains("positionCaseInsensitiveUTF8(display_name, ?) > 0"),
        "{sql}"
    );
    assert!(
        sql.contains("positionCaseInsensitiveUTF8(username, ?) > 0"),
        "{sql}"
    );
    Ok(())
}

#[test]
fn a_search_narrows_the_visitors_before_the_cap_keeps_its_200() -> R {
    let sql = people_sql(Order::default(), Some(&searching("ada")?));

    let filter = sql.find("positionCaseInsensitiveUTF8").ok_or("no filter")?;
    let cap = sql.find("LIMIT 200").ok_or("no cap")?;
    assert!(filter < cap, "{sql}");
    Ok(())
}

#[test]
fn a_search_is_bound_never_spliced_in() -> R {
    let needle = "ada'); DROP TABLE x; --";
    let sql = people_sql(Order::default(), Some(&searching(needle)?));

    assert!(!sql.contains(needle), "{sql}");
    assert_eq!(
        sql.matches('?').count(),
        7,
        "window, the name lookup's tenant, the needle twice, the join's tenant"
    );
    Ok(())
}

#[test]
fn no_search_leaves_the_visitors_unfiltered() {
    let sql = people_sql(Order::default(), None);

    assert!(!sql.contains("positionCaseInsensitiveUTF8"), "{sql}");
}
