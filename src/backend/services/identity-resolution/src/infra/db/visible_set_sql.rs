// INVARIANT: the same current-time union `subchart_repo` evaluates. A second
// rule would let a listing show a person the batch filter refuses to confirm.
pub(super) const CURRENT_VISIBLE_SET_CTE: &str = r"
        visible_set (person_id) AS (
            SELECT ?
            UNION
            SELECT viewed_person_id
            FROM visibility
            WHERE insight_tenant_id = ?
              AND viewer_person_id  = ?
              AND viewed_person_id  IS NOT NULL
              AND valid_from <= UTC_TIMESTAMP(6)
              AND (valid_to IS NULL OR valid_to > UTC_TIMESTAMP(6))
            UNION
            SELECT DISTINCT person_id
            FROM persons
            WHERE insight_tenant_id = ?
              AND (? OR EXISTS (
                  SELECT 1 FROM visibility
                  WHERE insight_tenant_id = ?
                    AND viewer_person_id  = ?
                    AND viewed_person_id  IS NULL
                    AND valid_from <= UTC_TIMESTAMP(6)
                    AND (valid_to IS NULL OR valid_to > UTC_TIMESTAMP(6))
              ))
            UNION
            SELECT oc.child_person_id
            FROM visible_set vs
            JOIN org_chart oc
              ON  oc.parent_person_id    = vs.person_id
              AND oc.insight_tenant_id   = ?
              AND oc.insight_source_type = ?
              AND oc.valid_from <= UTC_TIMESTAMP(6)
              AND (oc.valid_to IS NULL OR oc.valid_to > UTC_TIMESTAMP(6))
        )";
