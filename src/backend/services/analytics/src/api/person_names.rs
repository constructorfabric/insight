//! The mirrored identity rows a `product_usage` read model joins to name a person.

/// A `person_id → display_name / username` relation for one tenant, taking one
/// bound tenant parameter. Names come from the mirrored rows: a per-caller
/// profile lookup answers only for the caller's visible set, and these surfaces
/// are org-wide.
pub(crate) const NAMED_PERSONS: &str = "(\
  SELECT person_id, \
  coalesce(\
    nullIf(argMaxIf(value_effective, (created_at, id), value_type = 'display_name'), ''), \
    nullIf(trimBoth(concat(\
      coalesce(argMaxIf(value_effective, (created_at, id), value_type = 'first_name'), ''), \
      ' ', \
      coalesce(argMaxIf(value_effective, (created_at, id), value_type = 'last_name'), '') \
    )), '') \
  ) AS display_name, \
  nullIf(argMaxIf(value_effective, (created_at, id), value_type = 'username'), '') AS username \
  FROM identity.identity_persons \
  WHERE value_type IN ('display_name', 'first_name', 'last_name', 'username') \
  AND insight_tenant_id = toUUID(?) \
  GROUP BY person_id)";
