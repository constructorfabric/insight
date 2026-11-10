use std::collections::HashMap;

use uuid::Uuid;

use crate::infra::identity::{IdentityClient, RosterPerson};

pub(crate) type Roster = HashMap<Uuid, RosterPerson>;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Naming {
    pub(crate) display_name: String,
    pub(crate) username: String,
    pub(crate) email: Option<String>,
}

impl Naming {
    pub(crate) fn of(
        person: Option<&RosterPerson>,
        journal_display_name: String,
        journal_username: String,
    ) -> Self {
        let Some(person) = person else {
            return Self {
                display_name: journal_display_name,
                username: journal_username,
                email: None,
            };
        };

        Self {
            display_name: person.display_name.clone().unwrap_or_default(),
            username: person.username.clone().unwrap_or_default(),
            email: person.email.clone(),
        }
    }
}

pub(crate) async fn roster(
    identity: &IdentityClient,
    ids: &[Uuid],
    authorization: Option<&str>,
) -> Roster {
    match identity.tenant_people(ids, authorization).await {
        Ok(people) => people
            .into_iter()
            .map(|person| (person.person_id, person))
            .collect(),
        Err(error) => {
            tracing::warn!(error = %error, "naming people from the tenant roster failed");
            HashMap::new()
        }
    }
}

pub(crate) async fn matching(
    identity: &IdentityClient,
    needle: &str,
    authorization: Option<&str>,
) -> Vec<Uuid> {
    match identity.tenant_people_matching(needle, authorization).await {
        Ok(ids) => ids,
        Err(error) => {
            tracing::warn!(error = %error, "searching the tenant roster failed");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rostered(
        display_name: Option<&str>,
        username: Option<&str>,
        email: Option<&str>,
    ) -> RosterPerson {
        RosterPerson {
            person_id: Uuid::from_u128(1),
            display_name: display_name.map(str::to_owned),
            username: username.map(str::to_owned),
            email: email.map(str::to_owned),
        }
    }

    fn naming(display_name: &str, username: &str, email: Option<&str>) -> Naming {
        Naming {
            display_name: display_name.to_owned(),
            username: username.to_owned(),
            email: email.map(str::to_owned),
        }
    }

    #[test]
    fn the_roster_names_a_person_it_lists_and_the_journal_names_anyone_else() {
        let named = rostered(
            Some("Ada Example"),
            Some("aexample"),
            Some("ada@example.com"),
        );
        let nameless = rostered(None, None, Some("ada@example.com"));
        let cases = [
            (
                "on the roster",
                Some(&named),
                naming("Ada Example", "aexample", Some("ada@example.com")),
            ),
            (
                "on the roster with no name",
                Some(&nameless),
                naming("", "", Some("ada@example.com")),
            ),
            ("off the roster", None, naming("ada", "ada", None)),
        ];

        for (case, person, expected) in cases {
            assert_eq!(
                Naming::of(person, "ada".to_owned(), "ada".to_owned()),
                expected,
                "{case}"
            );
        }
    }

    #[tokio::test]
    async fn an_unreachable_roster_names_nobody_rather_than_failing() {
        let Ok(identity) = IdentityClient::new("http://127.0.0.1:1") else {
            unreachable!("the static identity url builds a client")
        };

        assert!(
            roster(&identity, &[Uuid::from_u128(1)], None)
                .await
                .is_empty()
        );
        assert!(matching(&identity, "ada", None).await.is_empty());
    }
}
