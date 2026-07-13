//! Profile domain: the `POST /v1/profiles` request/response DTOs and the
//! assembly of a person's observations into the response.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::infra::db::entities::persons;

/// Body of `POST /v1/profiles`. `value_type = "email"` matches across all
/// sources for the tenant; `value_type = "id"` matches a source-native account
/// id within one source instance (needs `insight_source_type` + `insight_source_id`).
#[derive(Debug, Deserialize, ToSchema)]
pub struct ResolveProfileCommand {
    pub value_type: String,
    pub value: String,
    /// Required when `value_type = "id"` — the source instance to scope to.
    #[serde(default)]
    pub insight_source_type: Option<String>,
    /// Required when `value_type = "id"`.
    #[serde(default)]
    pub insight_source_id: Option<Uuid>,
}

/// Response body of `POST /v1/profiles` — the resolved person's profile.
/// Attributes only for now; `ids[]` and the org tree (supervisor / parent /
/// subordinates) land in follow-up steps. Null fields are omitted from JSON.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProfileResponse {
    pub person_id: Uuid,
    pub insight_tenant_id: Uuid,
    // `email` and `display_name` are always present in JSON (null when absent),
    // matching the .NET contract (no `[JsonIgnore]` on these two).
    pub email: Option<String>,
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub department: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub division: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub employee_id: Option<String>,
}

// Marker traits the toolkit `OperationBuilder` requires (alongside `ToSchema`).
impl toolkit::api::api_dto::RequestApiDto for ResolveProfileCommand {}
impl toolkit::api::api_dto::ResponseApiDto for ProfileResponse {}

/// Collapse a person's observations to the current value per attribute — the
/// latest by `created_at` (per the .NET `ProfileAssembler`, ADR-0003) — and map
/// to the response DTO. `value_effective` is the DB's coalesced display value.
#[must_use]
pub fn assemble_profile(
    person_id: Uuid,
    tenant_id: Uuid,
    observations: Vec<persons::Model>,
) -> ProfileResponse {
    // Keep the latest observation per value_type (max created_at).
    let mut latest: HashMap<String, persons::Model> = HashMap::new();
    for obs in observations {
        match latest.get(&obs.value_type) {
            // Keep the current winner unless the new row is strictly newer.
            // Tie-break on `id` (matches the .NET `created_at DESC, id DESC`), so
            // the result is deterministic even when `created_at` values are equal
            // (common under batch backfill) and independent of DB row order.
            Some(prev) if (prev.created_at, prev.id) >= (obs.created_at, obs.id) => {}
            _ => {
                latest.insert(obs.value_type.clone(), obs);
            }
        }
    }

    let get = |value_type: &str| -> Option<String> {
        latest
            .get(value_type)
            .and_then(|m| m.value_effective.clone())
            .filter(|s| !s.trim().is_empty())
    };

    ProfileResponse {
        person_id,
        insight_tenant_id: tenant_id,
        email: get("email"),
        display_name: get("display_name"),
        first_name: get("first_name"),
        last_name: get("last_name"),
        department: get("department"),
        division: get("division"),
        job_title: get("job_title"),
        status: get("status"),
        username: get("username"),
        employee_id: get("employee_id"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::prelude::DateTime;

    /// Minimal observation carrying only the fields `assemble_profile` reads.
    fn obs(value_type: &str, value_effective: &str, created_at: DateTime) -> persons::Model {
        persons::Model {
            id: 0,
            value_type: value_type.to_owned(),
            insight_source_type: "test".to_owned(),
            insight_source_id: vec![0u8; 16],
            insight_tenant_id: vec![0u8; 16],
            value_id: None,
            value_full_text: None,
            value: None,
            value_effective: Some(value_effective.to_owned()),
            value_hash: None,
            person_id: vec![0u8; 16],
            author_person_id: vec![0u8; 16],
            reason: None,
            created_at,
        }
    }

    #[test]
    fn latest_observation_wins_per_value_type() -> anyhow::Result<()> {
        let older: DateTime = "2026-01-01T00:00:00".parse()?;
        let newer: DateTime = "2026-06-01T00:00:00".parse()?;
        let person_id = Uuid::from_u128(1);
        let tenant_id = Uuid::from_u128(2);

        let profile = assemble_profile(
            person_id,
            tenant_id,
            vec![
                obs("email", "old@example.com", older),
                obs("email", "new@example.com", newer), // newer wins
                obs("display_name", "Ann Smith", newer),
            ],
        );

        assert_eq!(profile.person_id, person_id);
        assert_eq!(profile.insight_tenant_id, tenant_id);
        assert_eq!(profile.email.as_deref(), Some("new@example.com"));
        assert_eq!(profile.display_name.as_deref(), Some("Ann Smith"));
        Ok(())
    }

    #[test]
    fn blank_values_and_missing_types_become_none() -> anyhow::Result<()> {
        let t: DateTime = "2026-01-01T00:00:00".parse()?;
        let profile = assemble_profile(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            vec![
                obs("email", "a@b.com", t),
                obs("department", "   ", t), // blank → None
            ],
        );

        assert_eq!(profile.email.as_deref(), Some("a@b.com"));
        assert_eq!(profile.department, None, "blank value_effective must be dropped");
        assert_eq!(profile.job_title, None, "absent value_type must be None");
        Ok(())
    }

    #[test]
    fn maps_all_known_attributes() -> anyhow::Result<()> {
        let t: DateTime = "2026-01-01T00:00:00".parse()?;
        let profile = assemble_profile(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            vec![
                obs("first_name", "Ann", t),
                obs("last_name", "Smith", t),
                obs("division", "R&D", t),
                obs("job_title", "Engineer", t),
                obs("status", "Active", t),
                obs("username", "asmith", t),
                obs("employee_id", "E1", t),
            ],
        );

        assert_eq!(profile.first_name.as_deref(), Some("Ann"));
        assert_eq!(profile.last_name.as_deref(), Some("Smith"));
        assert_eq!(profile.division.as_deref(), Some("R&D"));
        assert_eq!(profile.job_title.as_deref(), Some("Engineer"));
        assert_eq!(profile.status.as_deref(), Some("Active"));
        assert_eq!(profile.username.as_deref(), Some("asmith"));
        assert_eq!(profile.employee_id.as_deref(), Some("E1"));
        Ok(())
    }

    #[test]
    fn equal_created_at_breaks_tie_by_id_deterministically() -> anyhow::Result<()> {
        let t: DateTime = "2026-01-01T00:00:00".parse()?;
        let mut low = obs("email", "low-id@example.com", t);
        low.id = 10;
        let mut high = obs("email", "high-id@example.com", t);
        high.id = 20;

        // Highest id wins on equal created_at — regardless of input row order.
        let a = assemble_profile(Uuid::from_u128(1), Uuid::from_u128(2), vec![low.clone(), high.clone()]);
        let b = assemble_profile(Uuid::from_u128(1), Uuid::from_u128(2), vec![high, low]);

        assert_eq!(a.email.as_deref(), Some("high-id@example.com"));
        assert_eq!(b.email.as_deref(), Some("high-id@example.com"));
        Ok(())
    }
}
