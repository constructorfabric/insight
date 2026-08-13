//! Compact person card — the id→display projection operator surfaces embed
//! where a bare `person_id` would push the lookup onto every consumer (queue
//! candidates today, person search later). Deliberately flat: no supervisor,
//! no subordinates, no source ids — those belong to the full profile.

use std::collections::HashMap;

use uuid::Uuid;

use crate::domain::profile::latest_values;
use crate::infra::db::entities::persons;

/// The observation attributes a card is assembled from. The repo fetch filters
/// on this list so hydrating a page of candidates reads card rows only, not
/// every binding observation the journal holds for those persons.
pub const CARD_VALUE_TYPES: [&str; 7] = [
    "email",
    "username",
    "display_name",
    "first_name",
    "last_name",
    "job_title",
    "status",
];

#[derive(Debug, Clone)]
pub struct PersonCard {
    pub person_id: Uuid,
    pub email: Option<String>,
    /// Source-native handle (e.g. a git login) — often the only recognisable
    /// field of an identity no HR system has observed yet.
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub job_title: Option<String>,
    pub status: Option<String>,
}

impl PersonCard {
    /// The card for a person the journal holds no card attributes for — a
    /// candidate observed only through bindings still gets an entry, so a
    /// consumer can render the id rather than dropping the person.
    #[must_use]
    pub fn empty(person_id: Uuid) -> Self {
        Self {
            person_id,
            email: None,
            username: None,
            display_name: None,
            job_title: None,
            status: None,
        }
    }
}

/// Collapse card observations for MANY persons — rows as the repo returns
/// them, grouped here — to one card per person, current value per attribute
/// (same latest-wins rule the profile uses).
#[must_use]
pub fn assemble_cards(rows: Vec<persons::Model>) -> HashMap<Uuid, PersonCard> {
    let mut by_person: HashMap<Uuid, Vec<persons::Model>> = HashMap::new();
    for row in rows {
        let Ok(person_id) = Uuid::from_slice(&row.person_id) else {
            continue;
        };
        by_person.entry(person_id).or_default().push(row);
    }

    by_person
        .into_iter()
        .map(|(person_id, observations)| (person_id, card(person_id, observations)))
        .collect()
}

fn card(person_id: Uuid, observations: Vec<persons::Model>) -> PersonCard {
    let latest = latest_values(observations);
    let get = |value_type: &str| latest.get(value_type).cloned();

    // The profile splits display_name into first/last when those are missing;
    // a card wants the opposite fallback — compose a display name from the
    // parts when no display_name was ever observed.
    let display_name =
        get("display_name").or_else(|| compose_name(get("first_name"), get("last_name")));

    PersonCard {
        person_id,
        email: get("email"),
        username: get("username"),
        display_name,
        job_title: get("job_title"),
        status: get("status"),
    }
}

/// Cards for `ids` in the callers' order — a person the map has no card for
/// still appears, as the id alone. The one place the "absent card is not a
/// dropped person" rule lives; both the queue and the search render through it.
#[must_use]
pub fn in_requested_order(ids: &[Uuid], cards: &HashMap<Uuid, PersonCard>) -> Vec<PersonCard> {
    ids.iter()
        .map(|id| {
            cards
                .get(id)
                .cloned()
                .unwrap_or_else(|| PersonCard::empty(*id))
        })
        .collect()
}

fn compose_name(first: Option<String>, last: Option<String>) -> Option<String> {
    let joined = [first, last]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    (!joined.trim().is_empty()).then_some(joined)
}

#[cfg(test)]
mod tests {
    use sea_orm::prelude::DateTime;

    use super::*;

    fn obs(
        person: Uuid,
        value_type: &str,
        value: &str,
        created_at: &str,
    ) -> anyhow::Result<persons::Model> {
        Ok(persons::Model {
            id: 0,
            value_type: value_type.to_owned(),
            insight_source_type: "test".to_owned(),
            insight_source_id: vec![0u8; 16],
            insight_tenant_id: vec![0u8; 16],
            value_id: None,
            value_full_text: None,
            value: None,
            value_effective: Some(value.to_owned()),
            value_hash: None,
            person_id: person.as_bytes().to_vec(),
            author_person_id: vec![0u8; 16],
            reason: None,
            created_at: created_at.parse::<DateTime>()?,
        })
    }

    #[test]
    fn rows_group_per_person_and_latest_value_wins() -> anyhow::Result<()> {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let cards = assemble_cards(vec![
            obs(a, "email", "old@example.com", "2026-01-01T00:00:00")?,
            obs(a, "email", "new@example.com", "2026-02-01T00:00:00")?,
            obs(b, "display_name", "Bee Person", "2026-01-01T00:00:00")?,
        ]);

        assert_eq!(cards.len(), 2, "one card per person");
        assert_eq!(cards[&a].email.as_deref(), Some("new@example.com"));
        assert_eq!(cards[&b].display_name.as_deref(), Some("Bee Person"));
        assert_eq!(cards[&b].email, None);
        Ok(())
    }

    #[test]
    fn display_name_composes_from_parts_only_when_unobserved() -> anyhow::Result<()> {
        let named = Uuid::from_u128(1);
        let parts_only = Uuid::from_u128(2);
        let cards = assemble_cards(vec![
            obs(named, "display_name", "Full Name", "2026-01-01T00:00:00")?,
            obs(named, "first_name", "Other", "2026-01-01T00:00:00")?,
            obs(parts_only, "first_name", "First", "2026-01-01T00:00:00")?,
            obs(parts_only, "last_name", "Last", "2026-01-01T00:00:00")?,
        ]);

        assert_eq!(cards[&named].display_name.as_deref(), Some("Full Name"));
        assert_eq!(
            cards[&parts_only].display_name.as_deref(),
            Some("First Last")
        );
        Ok(())
    }

    #[test]
    fn a_lone_name_part_still_makes_a_display_name() -> anyhow::Result<()> {
        let person = Uuid::from_u128(1);
        let cards = assemble_cards(vec![obs(
            person,
            "last_name",
            "Mononym",
            "2026-01-01T00:00:00",
        )?]);

        assert_eq!(cards[&person].display_name.as_deref(), Some("Mononym"));
        Ok(())
    }
}
