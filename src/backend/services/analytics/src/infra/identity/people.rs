use serde::Deserialize;
use uuid::Uuid;

use super::IdentityClient;

// INVARIANT: identity refuses more ids than this in one lookup.
const MAX_IDS: usize = 200;

// INVARIANT: identity serves no more people than this in one page.
const MAX_MATCHES: usize = 500;

#[derive(Debug, Deserialize)]
struct PeopleList {
    items: Vec<RosterPerson>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RosterPerson {
    pub(crate) person_id: Uuid,
    pub(crate) display_name: Option<String>,
    pub(crate) username: Option<String>,
    pub(crate) email: Option<String>,
}

impl IdentityClient {
    pub(crate) async fn tenant_people(
        &self,
        person_ids: &[Uuid],
        authorization: Option<&str>,
    ) -> anyhow::Result<Vec<RosterPerson>> {
        let mut people = Vec::with_capacity(person_ids.len());
        for chunk in person_ids.chunks(MAX_IDS) {
            let ids = chunk
                .iter()
                .map(Uuid::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let limit = chunk.len().to_string();

            people.extend(
                self.tenant_roster(&[("ids", &ids), ("limit", &limit)], authorization)
                    .await?,
            );
        }

        Ok(people)
    }

    pub(crate) async fn tenant_people_matching(
        &self,
        needle: &str,
        authorization: Option<&str>,
    ) -> anyhow::Result<Vec<Uuid>> {
        let limit = MAX_MATCHES.to_string();
        let people = self
            .tenant_roster(&[("q", needle), ("limit", &limit)], authorization)
            .await?;

        Ok(people.into_iter().map(|person| person.person_id).collect())
    }

    async fn tenant_roster(
        &self,
        filter: &[(&str, &str)],
        authorization: Option<&str>,
    ) -> anyhow::Result<Vec<RosterPerson>> {
        let url = format!("{}/v1/people", self.base_url);
        let req = self
            .http
            .get(&url)
            .query(&[("visibility", "tenant")])
            .query(filter);

        let resp = Self::send(req, authorization, "tenant roster").await?;
        let page: PeopleList = resp.json().await?;
        Ok(page.items)
    }
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "a loopback setup failure should fail the test loudly"
)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::extract::Query;
    use axum::http::{HeaderMap, StatusCode, header::AUTHORIZATION};
    use axum::routing::get;

    use super::*;

    type Seen = Arc<Mutex<Vec<(Option<String>, HashMap<String, String>)>>>;

    async fn spawn_people(status: StatusCode, body: serde_json::Value) -> (String, Seen) {
        let seen: Seen = Arc::default();
        let record = Arc::clone(&seen);
        let app = Router::new().route(
            "/v1/people",
            get(
                move |headers: HeaderMap, Query(params): Query<HashMap<String, String>>| {
                    let record = Arc::clone(&record);
                    let body = body.clone();
                    async move {
                        let auth = headers
                            .get(AUTHORIZATION)
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_owned);
                        record.lock().unwrap().push((auth, params));
                        (status, axum::Json(body))
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), seen)
    }

    fn one_person(id: &str) -> serde_json::Value {
        serde_json::json!({
            "items": [{
                "person_id": id,
                "display_name": "Ada Example",
                "first_name": "Ada",
                "last_name": "Example",
                "username": "ada",
                "email": "ada@example.com",
                "attributes": {},
                "manager_person_id": null,
            }],
            "next_cursor": null,
        })
    }

    #[tokio::test]
    async fn tenant_people_reads_the_tenant_roster_for_exactly_these_ids_as_the_caller() {
        let ada = Uuid::from_u128(0xa);
        let bob = Uuid::from_u128(0xb);
        let (url, seen) = spawn_people(StatusCode::OK, one_person(&ada.to_string())).await;
        let client = IdentityClient::new(&url).unwrap();

        let people = client
            .tenant_people(&[ada, bob], Some("Bearer t"))
            .await
            .unwrap();

        assert_eq!(people.len(), 1);
        assert_eq!(people[0].person_id, ada);
        assert_eq!(people[0].display_name.as_deref(), Some("Ada Example"));
        assert_eq!(people[0].email.as_deref(), Some("ada@example.com"));
        let seen = seen.lock().unwrap();
        let (auth, params) = &seen[0];
        assert_eq!(auth.as_deref(), Some("Bearer t"));
        assert_eq!(params.get("visibility").map(String::as_str), Some("tenant"));
        assert_eq!(params.get("ids").cloned(), Some(format!("{ada},{bob}")));
        assert_eq!(params.get("limit").map(String::as_str), Some("2"));
    }

    #[tokio::test]
    async fn tenant_people_splits_more_ids_than_identity_takes_at_once() {
        let ids: Vec<Uuid> = (1..=201).map(Uuid::from_u128).collect();
        let (url, seen) = spawn_people(StatusCode::OK, serde_json::json!({"items": []})).await;
        let client = IdentityClient::new(&url).unwrap();

        client.tenant_people(&ids, None).await.unwrap();

        let asked: Vec<usize> = seen
            .lock()
            .unwrap()
            .iter()
            .map(|(_, params)| params["ids"].split(',').count())
            .collect();
        assert_eq!(asked, vec![200, 1]);
    }

    #[tokio::test]
    async fn nobody_to_name_asks_identity_nothing() {
        let (url, seen) = spawn_people(StatusCode::OK, serde_json::json!({"items": []})).await;
        let client = IdentityClient::new(&url).unwrap();

        let people = client.tenant_people(&[], None).await.unwrap();

        assert!(people.is_empty());
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_refused_roster_read_is_an_error() {
        let (url, _) = spawn_people(StatusCode::FORBIDDEN, serde_json::json!({})).await;
        let client = IdentityClient::new(&url).unwrap();

        assert!(
            client
                .tenant_people(&[Uuid::from_u128(1)], None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn tenant_people_matching_searches_the_tenant_roster_by_name() {
        let ada = Uuid::from_u128(0xa);
        let (url, seen) = spawn_people(StatusCode::OK, one_person(&ada.to_string())).await;
        let client = IdentityClient::new(&url).unwrap();

        let found = client
            .tenant_people_matching("Example", Some("Bearer t"))
            .await
            .unwrap();

        assert_eq!(found, vec![ada]);
        let seen = seen.lock().unwrap();
        let (auth, params) = &seen[0];
        assert_eq!(auth.as_deref(), Some("Bearer t"));
        assert_eq!(params.get("visibility").map(String::as_str), Some("tenant"));
        assert_eq!(params.get("q").map(String::as_str), Some("Example"));
    }
}
