//! Profile domain: the `POST /v1/profiles` request/response DTOs and the
//! assembly of a person's observations into the response.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::infra::db::entities::persons;
use crate::infra::db::persons_repo::SourceIdRow;

/// Body of `POST /v1/profiles`. `value_type = "email"` matches across all
/// sources for the tenant; `value_type = "id"` matches a source-native account
/// id within one source instance (needs `insight_source_type` + `insight_source_id`).
#[derive(Debug, Deserialize, ToSchema)]
pub struct ResolveProfileRequest {
    pub value_type: String,
    pub value: String,
    /// Required when `value_type = "id"` — the source instance to scope to.
    #[serde(default)]
    pub insight_source_type: Option<String>,
    /// Required when `value_type = "id"`.
    #[serde(default)]
    pub insight_source_id: Option<Uuid>,
}

/// Response body of `POST /v1/profiles` — the resolved person's profile:
/// current attributes, the org tree (`supervisor_*` / `parent_*` /
/// `subordinates[]`), and every current source-native id (`ids[]`). Null
/// attribute fields are omitted from JSON; `subordinates`/`ids` are always
/// present (empty when none), matching the .NET contract.
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
    // Org tree. `supervisor_*` and the legacy `parent_*` triple are both filled
    // from the single `org_chart` parent edge (matching the .NET assembler).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supervisor_email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supervisor_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_person_id: Option<Uuid>,
    /// Recursive subordinates subtree (direct reports and their reports), on the
    /// configured `org_chart` source. Always serialized (empty when none).
    pub subordinates: Vec<PersonResponse>,
    /// Every current source-native id for the person (one per source instance).
    /// Always serialized — an empty array when the person has no ids — matching
    /// the .NET contract (unlike the attributes above, which are omitted).
    pub ids: Vec<ProfileIdEntry>,
}

/// A person node in the org tree (subordinate of a profile), matching the .NET
/// `PersonResponse`. Unlike `ProfileResponse`, the attribute fields are plain
/// strings (empty when absent, not omitted) and the `supervisor_*`/`parent_*`
/// fields serialize as `null` rather than being dropped.
#[derive(Debug, Serialize, ToSchema)]
pub struct PersonResponse {
    pub person_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub first_name: String,
    pub last_name: String,
    pub department: String,
    pub division: String,
    pub job_title: String,
    pub status: String,
    pub supervisor_email: Option<String>,
    pub supervisor_name: Option<String>,
    pub parent_email: Option<String>,
    pub parent_id: Option<String>,
    pub parent_person_id: Option<Uuid>,
    // `no_recursion`: this field makes `PersonResponse` self-referential; without
    // it utoipa's schema generation recurses forever and overflows the stack at
    // route registration. Affects only the OpenAPI schema (emits a `$ref`), not
    // the serialized JSON — the response still carries the full nested tree.
    #[schema(no_recursion)]
    pub subordinates: Vec<PersonResponse>,
}

/// One source-native account id bound to the person — the latest
/// `value_type='id'` observation per source instance. Ported from the .NET
/// `ProfileIdEntry`.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProfileIdEntry {
    pub insight_source_type: String,
    pub insight_source_id: Uuid,
    pub value: String,
}

/// The parent (a.k.a. supervisor) edge resolved into the fields written onto
/// the response. Both the `supervisor_*` and legacy `parent_*` fields come from
/// this single projection (matching the .NET `PersonAssembler`). `None` leaves
/// every parent field null.
pub struct ParentProjection {
    pub person_id: Uuid,
    pub email: Option<String>,
    pub display_name: Option<String>,
    /// Parent's source-native id on the edge's source instance (→ `parent_id`).
    pub source_native_id: Option<String>,
}

// Marker traits the toolkit `OperationBuilder` requires (alongside `ToSchema`).
impl toolkit::api::api_dto::RequestApiDto for ResolveProfileRequest {}
impl toolkit::api::api_dto::ResponseApiDto for ProfileResponse {}
impl toolkit::api::api_dto::ResponseApiDto for PersonResponse {}

/// Collapse a person's observations to the current value per attribute — the
/// latest by `created_at` (per the .NET `ProfileAssembler`, ADR-0003) — and map
/// to the response DTO. `value_effective` is the DB's coalesced display value.
#[must_use]
pub fn assemble_profile(
    person_id: Uuid,
    tenant_id: Uuid,
    observations: Vec<persons::Model>,
    source_ids: Vec<SourceIdRow>,
    parent: Option<ParentProjection>,
    subordinates: Vec<PersonResponse>,
) -> ProfileResponse {
    let latest = latest_values(observations);
    let get = |value_type: &str| latest.get(value_type).cloned();

    // Display-name fallback: derive first/last from display_name only when
    // neither is observed (matches the .NET `DisplayNameSplitter` path).
    let display_name = get("display_name");
    let mut first_name = get("first_name");
    let mut last_name = get("last_name");
    if first_name.is_none()
        && last_name.is_none()
        && let Some(dn) = display_name.as_deref()
    {
        let (first, last) = split_display_name(dn);
        first_name = non_blank(first);
        last_name = non_blank(last);
    }

    // Map repo rows to API entries — the DB layer stays free of API DTOs.
    let ids = source_ids
        .into_iter()
        .map(|s| ProfileIdEntry {
            insight_source_type: s.source_type,
            insight_source_id: s.source_id,
            value: s.value,
        })
        .collect();

    // Both `supervisor_*` and legacy `parent_*` are filled from the one edge.
    let (supervisor_email, supervisor_name, parent_email, parent_id, parent_person_id) =
        match parent {
            Some(p) => (
                p.email.clone(),
                p.display_name,
                p.email,
                p.source_native_id.and_then(non_blank),
                Some(p.person_id),
            ),
            None => (None, None, None, None, None),
        };

    ProfileResponse {
        person_id,
        insight_tenant_id: tenant_id,
        email: get("email"),
        display_name,
        first_name,
        last_name,
        department: get("department"),
        division: get("division"),
        job_title: get("job_title"),
        status: get("status"),
        username: get("username"),
        employee_id: get("employee_id"),
        supervisor_email,
        supervisor_name,
        parent_email,
        parent_id,
        parent_person_id,
        subordinates,
        ids,
    }
}

/// Assemble a subordinate `PersonResponse` from its observations, parent edge,
/// and already-hydrated child subtree. Mirrors the .NET `PersonAssembler`:
/// attribute fields default to the empty string (not omitted), and the
/// display-name split fallback applies here too.
#[must_use]
pub fn assemble_person(
    person_id: Uuid,
    observations: Vec<persons::Model>,
    parent: Option<ParentProjection>,
    subordinates: Vec<PersonResponse>,
) -> PersonResponse {
    let latest = latest_values(observations);
    let get = |value_type: &str| latest.get(value_type).cloned().unwrap_or_default();

    let display_name = get("display_name");
    let mut first_name = get("first_name");
    let mut last_name = get("last_name");
    if first_name.is_empty() && last_name.is_empty() && !display_name.is_empty() {
        (first_name, last_name) = split_display_name(&display_name);
    }

    let (supervisor_email, supervisor_name, parent_email, parent_id, parent_person_id) =
        match parent {
            Some(p) => (
                p.email.clone(),
                p.display_name,
                p.email,
                p.source_native_id.and_then(non_blank),
                Some(p.person_id),
            ),
            None => (None, None, None, None, None),
        };

    PersonResponse {
        person_id,
        email: get("email"),
        display_name,
        first_name,
        last_name,
        department: get("department"),
        division: get("division"),
        job_title: get("job_title"),
        status: get("status"),
        supervisor_email,
        supervisor_name,
        parent_email,
        parent_id,
        parent_person_id,
        subordinates,
    }
}

/// Collapse observations to the current `value_effective` per `value_type` —
/// latest by `(created_at, id)`, blank values dropped. Shared by profile
/// assembly and parent-edge projection.
#[must_use]
pub fn latest_values(observations: Vec<persons::Model>) -> HashMap<String, String> {
    let mut latest: HashMap<String, persons::Model> = HashMap::new();
    for obs in observations {
        match latest.get(&obs.value_type) {
            // Tie-break on `id` (matches the .NET `created_at DESC, id DESC`), so
            // the result is deterministic even when `created_at` values are equal
            // (common under batch backfill) and independent of DB row order.
            Some(prev) if (prev.created_at, prev.id) >= (obs.created_at, obs.id) => {}
            _ => {
                latest.insert(obs.value_type.clone(), obs);
            }
        }
    }
    latest
        .into_iter()
        .filter_map(|(k, m)| {
            // Keep the raw value; trim is only the emptiness test. .NET does not
            // trim (NullIfBlank / GetValueOrDefault return the value verbatim),
            // so leading/trailing whitespace in source data must survive.
            let value = m.value_effective?;
            (!value.trim().is_empty()).then_some((k, value))
        })
        .collect()
}

/// Best-effort split of a display name into `(first, last)` when dedicated
/// observations are missing. Ported from the .NET `DisplayNameSplitter`:
/// `"Last, First"` (comma) → `(after, before)`; `"First Rest"` (space) →
/// `(before, rest)`; single token → `(token, "")`; blank → `("", "")`.
fn split_display_name(display_name: &str) -> (String, String) {
    let trimmed = display_name.trim();
    if trimmed.is_empty() {
        return (String::new(), String::new());
    }
    if let Some((before, after)) = trimmed.split_once(',') {
        return (after.trim().to_owned(), before.trim().to_owned());
    }
    if let Some((before, after)) = trimmed.split_once(' ') {
        return (before.trim().to_owned(), after.trim().to_owned());
    }
    (trimmed.to_owned(), String::new())
}

/// `None` when the string is blank, else `Some(s)`.
fn non_blank(s: String) -> Option<String> {
    if s.trim().is_empty() { None } else { Some(s) }
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
            vec![],
            None,
            Vec::new(),
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
            vec![],
            None,
            Vec::new(),
        );

        assert_eq!(profile.email.as_deref(), Some("a@b.com"));
        assert_eq!(
            profile.department, None,
            "blank value_effective must be dropped"
        );
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
            vec![],
            None,
            Vec::new(),
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
        let a = assemble_profile(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            vec![low.clone(), high.clone()],
            vec![],
            None,
            Vec::new(),
        );
        let b = assemble_profile(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            vec![high, low],
            vec![],
            None,
            Vec::new(),
        );

        assert_eq!(a.email.as_deref(), Some("high-id@example.com"));
        assert_eq!(b.email.as_deref(), Some("high-id@example.com"));
        Ok(())
    }

    #[test]
    fn maps_source_ids_into_response_preserving_order() -> anyhow::Result<()> {
        let t: DateTime = "2026-01-01T00:00:00".parse()?;
        let profile = assemble_profile(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            vec![obs("email", "a@b.com", t)],
            vec![
                SourceIdRow {
                    source_type: "github".to_owned(),
                    source_id: Uuid::from_u128(7),
                    value: "octocat".to_owned(),
                },
                SourceIdRow {
                    source_type: "slack".to_owned(),
                    source_id: Uuid::from_u128(8),
                    value: "U123".to_owned(),
                },
            ],
            None,
            Vec::new(),
        );

        assert_eq!(profile.ids.len(), 2);
        assert_eq!(profile.ids[0].insight_source_type, "github");
        assert_eq!(profile.ids[0].insight_source_id, Uuid::from_u128(7));
        assert_eq!(profile.ids[0].value, "octocat");
        assert_eq!(profile.ids[1].value, "U123");
        Ok(())
    }

    #[test]
    fn ids_default_to_empty_array() -> anyhow::Result<()> {
        let t: DateTime = "2026-01-01T00:00:00".parse()?;
        let profile = assemble_profile(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            vec![obs("email", "a@b.com", t)],
            vec![],
            None,
            Vec::new(),
        );
        assert!(profile.ids.is_empty());
        Ok(())
    }

    #[test]
    fn parent_projection_fills_supervisor_and_parent_fields() -> anyhow::Result<()> {
        let t: DateTime = "2026-01-01T00:00:00".parse()?;
        let parent = ParentProjection {
            person_id: Uuid::from_u128(9),
            email: Some("boss@example.com".to_owned()),
            display_name: Some("Big Boss".to_owned()),
            source_native_id: Some("E42".to_owned()),
        };
        let profile = assemble_profile(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            vec![obs("email", "a@b.com", t)],
            vec![],
            Some(parent),
            Vec::new(),
        );

        // supervisor_* and legacy parent_* mirror the single edge.
        assert_eq!(
            profile.supervisor_email.as_deref(),
            Some("boss@example.com")
        );
        assert_eq!(profile.supervisor_name.as_deref(), Some("Big Boss"));
        assert_eq!(profile.parent_email.as_deref(), Some("boss@example.com"));
        assert_eq!(profile.parent_id.as_deref(), Some("E42"));
        assert_eq!(profile.parent_person_id, Some(Uuid::from_u128(9)));
        Ok(())
    }

    #[test]
    fn no_parent_leaves_org_fields_none() -> anyhow::Result<()> {
        let t: DateTime = "2026-01-01T00:00:00".parse()?;
        let profile = assemble_profile(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            vec![obs("email", "a@b.com", t)],
            vec![],
            None,
            Vec::new(),
        );
        assert_eq!(profile.supervisor_email, None);
        assert_eq!(profile.parent_person_id, None);
        Ok(())
    }

    #[test]
    fn display_name_split_fallback_only_when_first_last_absent() -> anyhow::Result<()> {
        let t: DateTime = "2026-01-01T00:00:00".parse()?;

        // "First Last" split when neither name is observed.
        let split = assemble_profile(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            vec![obs("display_name", "Ann Smith", t)],
            vec![],
            None,
            Vec::new(),
        );
        assert_eq!(split.first_name.as_deref(), Some("Ann"));
        assert_eq!(split.last_name.as_deref(), Some("Smith"));

        // "Last, First" comma form.
        let comma = assemble_profile(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            vec![obs("display_name", "Smith, Ann", t)],
            vec![],
            None,
            Vec::new(),
        );
        assert_eq!(comma.first_name.as_deref(), Some("Ann"));
        assert_eq!(comma.last_name.as_deref(), Some("Smith"));

        // An explicit first_name suppresses the fallback entirely.
        let explicit = assemble_profile(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            vec![
                obs("display_name", "Ann Smith", t),
                obs("first_name", "Annie", t),
            ],
            vec![],
            None,
            Vec::new(),
        );
        assert_eq!(explicit.first_name.as_deref(), Some("Annie"));
        assert_eq!(explicit.last_name, None);
        Ok(())
    }

    #[test]
    fn assemble_person_uses_empty_strings_and_carries_subordinates() -> anyhow::Result<()> {
        let t: DateTime = "2026-01-01T00:00:00".parse()?;
        let leaf = assemble_person(
            Uuid::from_u128(30),
            vec![obs("email", "leaf@example.com", t)],
            None,
            Vec::new(),
        );
        // Absent attributes are empty strings (not omitted), per .NET PersonResponse.
        assert_eq!(leaf.email, "leaf@example.com");
        assert_eq!(leaf.department, "");
        assert_eq!(leaf.first_name, "");
        assert!(leaf.subordinates.is_empty());

        // Display-name split fallback applies to person nodes too.
        let mid = assemble_person(
            Uuid::from_u128(20),
            vec![obs("display_name", "Mid Manager", t)],
            None,
            vec![leaf],
        );
        assert_eq!(mid.first_name, "Mid");
        assert_eq!(mid.last_name, "Manager");
        assert_eq!(mid.subordinates.len(), 1);
        assert_eq!(mid.subordinates[0].person_id, Uuid::from_u128(30));
        Ok(())
    }

    #[test]
    fn profile_carries_subordinates() -> anyhow::Result<()> {
        let t: DateTime = "2026-01-01T00:00:00".parse()?;
        let sub = assemble_person(
            Uuid::from_u128(30),
            vec![obs("email", "s@e.com", t)],
            None,
            Vec::new(),
        );
        let profile = assemble_profile(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            vec![obs("email", "a@b.com", t)],
            vec![],
            None,
            vec![sub],
        );
        assert_eq!(profile.subordinates.len(), 1);
        assert_eq!(profile.subordinates[0].person_id, Uuid::from_u128(30));
        Ok(())
    }

    #[test]
    fn values_are_not_trimmed_only_dropped_when_blank() -> anyhow::Result<()> {
        let t: DateTime = "2026-01-01T00:00:00".parse()?;
        let profile = assemble_profile(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            vec![
                obs("department", " Engineering ", t), // whitespace preserved
                obs("division", "   ", t),             // blank → dropped
            ],
            vec![],
            None,
            Vec::new(),
        );
        // .NET returns the value verbatim (no TRIM); only blank collapses to None.
        assert_eq!(profile.department.as_deref(), Some(" Engineering "));
        assert_eq!(profile.division, None);
        Ok(())
    }
}
