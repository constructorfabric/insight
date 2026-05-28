namespace Insight.Identity.Infrastructure.MariaDb;

/// <summary>
/// SQL for the <c>persons-seed</c> operation. The read queries feed the
/// C# resolver (known-account bindings, latest-email map); the write
/// queries apply the resolved assignments (INSERT IGNORE observations)
/// and rebuild the two derived caches (<c>account_person_map</c>,
/// <c>org_chart</c>) tenant-scoped.
///
/// The rebuilds are ported from <c>seed-persons-from-identity-input.py</c>
/// but swap the whole-table RENAME swap for a tenant-scoped
/// DELETE+INSERT pair (run inside one transaction by the repository) —
/// the RENAME trick is all-tenant and does not fit tenant isolation.
/// </summary>
internal static class SqlPersonsSeed
{
    /// <summary>
    /// Current <c>source_account_id → person_id</c> bindings: the
    /// latest <c>value_type='id'</c> observation per
    /// <c>(source_type, source_id, value_id)</c> in the tenant.
    /// </summary>
    public const string KnownAccountBindings = """
        WITH ranked AS (
            SELECT
                insight_source_type,
                insight_source_id,
                value_id AS source_account_id,
                person_id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, insight_source_type, insight_source_id, value_id
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE value_type = 'id'
              AND value_id IS NOT NULL
              AND insight_tenant_id = @tenant_id
        )
        SELECT insight_source_type, insight_source_id, source_account_id, person_id
        FROM ranked
        WHERE rn = 1
        """;

    /// <summary>
    /// Current email → person_id map: the latest
    /// <c>value_type='email'</c> observation per (tenant, email). One
    /// person per email — corrupted multi-person emails resolve to the
    /// most recently observed person. Case-insensitivity is handled by
    /// the <c>utf8mb4_unicode_ci</c> collation on <c>value_id</c>
    /// (ADR-0011): the <c>PARTITION BY value_id</c> collapses
    /// case-variants into one partition, so no LOWER/TRIM is applied.
    /// </summary>
    public const string LatestEmailToPerson = """
        WITH ranked AS (
            SELECT
                value_id AS email,
                person_id,
                ROW_NUMBER() OVER (
                    PARTITION BY insight_tenant_id, value_id
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM persons
            WHERE value_type = 'email'
              AND value_id IS NOT NULL
              AND value_id != ''
              AND insight_tenant_id = @tenant_id
        )
        SELECT email, person_id
        FROM ranked
        WHERE rn = 1
        """;

    /// <summary>
    /// Idempotent observation insert. The UNIQUE key
    /// <c>uq_person_observation</c> dedups a re-emitted identical
    /// observation; INSERT IGNORE swallows the duplicate-key error so a
    /// re-seed is safe.
    /// </summary>
    public const string InsertObservation = """
        INSERT IGNORE INTO persons
            (value_type, insight_source_type, insight_source_id, insight_tenant_id,
             value_id, value_full_text, value,
             person_id, author_person_id, reason, created_at)
        VALUES
            (@value_type, @source_type, @source_id, @tenant_id,
             @value_id, @value_full_text, @value,
             @person_id, @author_person_id, @reason, @created_at)
        """;

    // ── account_person_map rebuild (tenant-scoped) ──────────────────

    public const string DeleteAccountPersonMapForTenant = """
        DELETE FROM account_person_map WHERE insight_tenant_id = @tenant_id
        """;

    /// <summary>
    /// Rebuild the tenant's SCD2 account→person bindings from
    /// <c>persons</c>. <c>valid_to</c> of each row is the
    /// <c>created_at</c> of the next observation on the same account
    /// (LEAD), NULL for the most recent.
    /// </summary>
    public const string InsertAccountPersonMapForTenant = """
        INSERT INTO account_person_map
            (insight_tenant_id, insight_source_type, insight_source_id, source_account_id,
             person_id, author_person_id, reason, valid_from, valid_to)
        SELECT
            insight_tenant_id,
            insight_source_type,
            insight_source_id,
            value_id AS source_account_id,
            person_id,
            author_person_id,
            reason,
            created_at AS valid_from,
            LEAD(created_at) OVER (
                PARTITION BY insight_tenant_id, insight_source_type,
                             insight_source_id, value_id
                ORDER BY created_at
            ) AS valid_to
        FROM persons
        WHERE value_type = 'id'
          AND value_id IS NOT NULL
          AND insight_tenant_id = @tenant_id
        """;

    // ── org_chart rebuild (tenant-scoped) ───────────────────────────

    public const string DeleteOrgChartForTenant = """
        DELETE FROM org_chart WHERE insight_tenant_id = @tenant_id
        """;

    /// <summary>
    /// Rebuild the tenant's parent→child edges from <c>persons</c>.
    /// Ported verbatim from the Python seeder step 9 (active-interval
    /// computation + two-source priority) with an
    /// <c>insight_tenant_id = @tenant_id</c> filter added to every
    /// <c>persons</c> read. Source 1 (resolved parent_person_id) wins
    /// over Source 2 (parent_email→email JOIN) via the NOT EXISTS guard.
    /// </summary>
    public const string InsertOrgChartForTenant = """
        INSERT INTO org_chart
            (insight_tenant_id, insight_source_type, insight_source_id,
             child_person_id, parent_person_id,
             author_person_id, reason, valid_from, valid_to)
        WITH
        state_log AS (
            SELECT
                insight_tenant_id, insight_source_type, insight_source_id, person_id,
                created_at, id,
                CASE
                    WHEN value_full_text IN ('Inactive', 'Terminated', 'inactive', 'terminated')
                        THEN 0 ELSE 1
                END AS is_active,
                LAG(CASE
                    WHEN value_full_text IN ('Inactive', 'Terminated', 'inactive', 'terminated')
                        THEN 0 ELSE 1
                END) OVER (
                    PARTITION BY insight_tenant_id, insight_source_type, insight_source_id, person_id
                    ORDER BY created_at, id
                ) AS prev_is_active
            FROM persons
            WHERE value_type = 'status'
              AND value_full_text IS NOT NULL
              AND insight_tenant_id = @tenant_id
        ),
        state_transitions AS (
            SELECT
                insight_tenant_id, insight_source_type, insight_source_id, person_id,
                created_at, id, is_active,
                LEAD(created_at) OVER (
                    PARTITION BY insight_tenant_id, insight_source_type, insight_source_id, person_id
                    ORDER BY created_at, id
                ) AS next_transition_at
            FROM state_log
            WHERE prev_is_active IS NULL OR prev_is_active <> is_active
        ),
        active_intervals AS (
            SELECT
                insight_tenant_id, insight_source_type, insight_source_id, person_id,
                created_at         AS interval_start,
                next_transition_at AS interval_end
            FROM state_transitions
            WHERE is_active = 1
        ),
        default_active AS (
            SELECT DISTINCT
                pe.insight_tenant_id, pe.insight_source_type, pe.insight_source_id, pe.person_id,
                CAST('1970-01-01 00:00:00.000000' AS DATETIME(6)) AS interval_start,
                CAST(NULL AS DATETIME(6)) AS interval_end
            FROM persons pe
            WHERE pe.value_type = 'parent_email'
              AND pe.value_id IS NOT NULL
              AND pe.insight_tenant_id = @tenant_id
              AND NOT EXISTS (
                  SELECT 1 FROM persons s
                  WHERE s.insight_tenant_id   = pe.insight_tenant_id
                    AND s.insight_source_type = pe.insight_source_type
                    AND s.insight_source_id   = pe.insight_source_id
                    AND s.person_id           = pe.person_id
                    AND s.value_type          = 'status'
              )
        ),
        all_active AS (
            SELECT * FROM active_intervals
            UNION ALL
            SELECT * FROM default_active
        ),
        pe_periods AS (
            SELECT
                pe.insight_tenant_id, pe.insight_source_type, pe.insight_source_id,
                pe.person_id AS child_person_id,
                pe.value_id AS parent_email,
                pe.author_person_id, pe.reason,
                pe.created_at AS pe_from,
                LEAD(pe.created_at) OVER (
                    PARTITION BY pe.insight_tenant_id, pe.insight_source_type,
                                 pe.insight_source_id, pe.person_id
                    ORDER BY pe.created_at, pe.id
                ) AS pe_to
            FROM persons pe
            WHERE pe.value_type = 'parent_email'
              AND pe.value_id IS NOT NULL
              AND pe.insight_tenant_id = @tenant_id
        ),
        email_to_person AS (
            SELECT
                p.insight_tenant_id, p.value_id, p.person_id,
                ROW_NUMBER() OVER (
                    PARTITION BY p.insight_tenant_id, p.value_id
                    ORDER BY p.created_at DESC, p.id DESC
                ) AS rn
            FROM persons p
            WHERE p.value_type = 'email'
              AND p.value_id IS NOT NULL
              AND p.insight_tenant_id = @tenant_id
        )

        SELECT
            insight_tenant_id, insight_source_type, insight_source_id,
            person_id                                       AS child_person_id,
            UNHEX(REPLACE(value_id, '-', ''))               AS parent_person_id,
            author_person_id, reason,
            created_at                                      AS valid_from,
            LEAD(created_at) OVER (
                PARTITION BY insight_tenant_id, insight_source_type,
                             insight_source_id, person_id
                ORDER BY created_at
            )                                               AS valid_to
        FROM persons
        WHERE value_type = 'parent_person_id'
          AND value_id IS NOT NULL
          AND insight_tenant_id = @tenant_id
          AND value_id REGEXP '^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$'
          AND HEX(person_id) <> REPLACE(value_id, '-', '')

        UNION ALL

        SELECT
            pe.insight_tenant_id, pe.insight_source_type, pe.insight_source_id,
            pe.child_person_id,
            parent.person_id                                AS parent_person_id,
            pe.author_person_id, pe.reason,
            GREATEST(pe.pe_from, ai.interval_start)         AS valid_from,
            CASE
                WHEN pe.pe_to IS NULL AND ai.interval_end IS NULL THEN NULL
                WHEN pe.pe_to        IS NULL                      THEN ai.interval_end
                WHEN ai.interval_end IS NULL                      THEN pe.pe_to
                ELSE LEAST(pe.pe_to, ai.interval_end)
            END                                             AS valid_to
        FROM pe_periods pe
        INNER JOIN email_to_person parent
            ON parent.insight_tenant_id = pe.insight_tenant_id
           AND parent.value_id          = pe.parent_email
           AND parent.rn                = 1
        INNER JOIN all_active ai
            ON ai.insight_tenant_id   = pe.insight_tenant_id
           AND ai.insight_source_type = pe.insight_source_type
           AND ai.insight_source_id   = pe.insight_source_id
           AND ai.person_id           = pe.child_person_id
           AND ai.interval_start < COALESCE(pe.pe_to, '9999-12-31 23:59:59.999999')
           AND COALESCE(ai.interval_end, '9999-12-31 23:59:59.999999') > pe.pe_from
        WHERE parent.person_id <> pe.child_person_id
          AND NOT EXISTS (
              SELECT 1 FROM persons ppi
              WHERE ppi.insight_tenant_id   = pe.insight_tenant_id
                AND ppi.person_id           = pe.child_person_id
                AND ppi.insight_source_type = pe.insight_source_type
                AND ppi.insight_source_id   = pe.insight_source_id
                AND ppi.value_type          = 'parent_person_id'
                AND ppi.value_id IS NOT NULL
          )
        """;
}
