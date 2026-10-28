use uuid::Uuid;

use super::*;
use crate::domain::access::PersonId;
use crate::store::definitions::visibility::assert_bound;

fn caller() -> Caller {
    Caller::member(PersonId::new(Uuid::from_u128(7)))
}

#[test]
fn listing_and_counting_pins_keep_only_dashboards_the_caller_sees() {
    let (visible, _) = dashboard_visible(caller());

    for template in [PINS_OF, COUNT_PINS] {
        let statement = visible_pins(template, caller());

        assert!(statement.sql.contains(&visible), "{}", statement.sql);
        assert!(
            statement.sql.contains("dashboard_pins.person = ?"),
            "{}",
            statement.sql
        );
        assert_bound(&statement);
    }
}

#[test]
fn a_pin_lands_only_on_a_dashboard_the_caller_sees() {
    let dashboard = DefinitionId::parse("delivery").unwrap_or_else(|error| panic!("{error}"));
    let (visible, _) = dashboard_visible(caller());

    let statement = insert_pin(caller(), &dashboard);

    assert!(statement.sql.contains(&visible), "{}", statement.sql);
    assert!(
        statement.sql.contains("FROM dashboards"),
        "{}",
        statement.sql
    );
    assert_bound(&statement);
}
