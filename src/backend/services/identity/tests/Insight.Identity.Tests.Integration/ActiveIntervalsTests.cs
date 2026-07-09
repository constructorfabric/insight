using FluentAssertions;
using MySqlConnector;
using Xunit;

namespace Insight.Identity.Tests.Integration;

/// <summary>
/// Regression tests for the SCD2 active-intervals CTE used by
/// `seed-persons-from-identity-input.py` step 9 (ADR-0010 Phase 1 of
/// constructorfabric/insight#348).
///
/// These tests target the algorithmic core of the rebuild SQL — the
/// `state_log` / `state_transitions` / `active_intervals` CTE chain
/// that decides when a parent->child edge is "currently open" vs
/// "closed at the deactivation moment". The full rebuild also
/// joins parent_email -> email and produces org_chart rows;
/// `OrgChartTests.cs` covers the reader side. This file
/// drills into the SCD2 logic specifically because the cf-sdlc-pr-review
/// on PR #477 (Finding F1) caught a window-over-filtered-rows bug
/// here that the kind-cluster run did not surface (the production
/// BambooHR snapshot had no Active->Inactive transitions).
///
/// IMPORTANT: the CTE string in <see cref="ActiveIntervalsCte"/>
/// MUST stay in sync with the same CTE in
/// `src/backend/services/identity/seed/seed-persons-from-identity-input.py`
/// step 9. There is no shared SQL resource in Phase 1; drift is
/// guarded only by the seeder being the production path and these
/// tests being the regression net. Phase 1.5 follow-up: extract the
/// rebuild SQL into an embedded resource both Python and C# load.
/// </summary>
[Collection(MariaDbCollection.Name)]
public sealed class ActiveIntervalsTests : IAsyncLifetime
{
    private static readonly Guid TenantId       = Guid.Parse("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    private static readonly Guid BambooSourceId = Guid.Parse("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
    private static readonly Guid PersonId       = Guid.Parse("cccccccc-cccc-cccc-cccc-cccccccccccc");
    private static readonly Guid AuthorPersonId = Guid.Empty;
    private static readonly string[] OnlyPrimaryIndex = { "PRIMARY" };

    private readonly MariaDbFixture _fixture;

    public ActiveIntervalsTests(MariaDbFixture fixture) => _fixture = fixture;

    public async Task InitializeAsync() => await _fixture.ResetAsync().ConfigureAwait(false);
    public Task DisposeAsync() => Task.CompletedTask;

    // ── F1 regression: Active -> Inactive must close interval ──────

    [Fact]
    public async Task ActiveIntervals_closes_interval_at_deactivation_for_simple_transition()
    {
        // The exact scenario the F1 bug missed: an employee was Active,
        // then went Inactive. The buggy SQL kept interval_end = NULL
        // because LEAD operated over Active-only rows after WHERE
        // is_active = 1. Fixed by moving LEAD into state_transitions
        // (before the is_active filter).
        var t0 = new DateTime(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc);
        var t2 = new DateTime(2026, 6, 1, 0, 0, 0, DateTimeKind.Utc);
        await InsertStatusAsync(t0, "Active").ConfigureAwait(false);
        await InsertStatusAsync(t2, "Inactive").ConfigureAwait(false);

        var intervals = await QueryActiveIntervalsAsync().ConfigureAwait(false);

        intervals.Should().HaveCount(1);
        intervals[0].Start.Should().Be(t0);
        intervals[0].End.Should().Be(t2);   // ← buggy code returned NULL here
    }

    // -------------------------------------------------------------
    // The two tests below are [Fact(Skip = ...)] because they require
    // multiple status observations on the same partition — which
    // `persons.uq_person_observation` UNIQUE on
    //   (tenant, person, source_type, source_id, value_type, value_hash)
    // currently blocks. `value_hash` is SHA2(value_effective), so two
    // `status='Active'` observations collide and the second one is
    // dropped by the seeder's `INSERT IGNORE`.
    //
    // The SCD2 logic in the active-intervals CTE handles these shapes
    // correctly (verified by reasoning, can be re-verified once the
    // schema allows the data). The blocker is the UNIQUE constraint,
    // tracked as a separate follow-up: drop UNIQUE on value_hash so
    // re-emission of the same value at a different `created_at`
    // produces a fresh row (the persons table is designed as an
    // append-only observation log; the current UNIQUE prevents
    // recording state-revert transitions which is wrong).
    //
    // Remove the Skip attribute when the schema follow-up lands. No
    // CTE changes should be needed — the F1 fix in step 9 already
    // handles these scenarios correctly.
    // -------------------------------------------------------------

    [Fact(Skip = "Blocked by persons UNIQUE on value_hash — see schema-fix follow-up; F1 fix already supports this scenario in the CTE.")]
    public async Task ActiveIntervals_yields_two_intervals_for_reactivation()
    {
        // Active(T0) → Inactive(T2) → Active(T3) — the canonical
        // re-activation scenario from the design discussion. Expected:
        // two intervals, [T0, T2) and [T3, NULL).
        var t0 = new DateTime(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc);
        var t2 = new DateTime(2026, 6, 1, 0, 0, 0, DateTimeKind.Utc);
        var t3 = new DateTime(2026, 8, 1, 0, 0, 0, DateTimeKind.Utc);
        await InsertStatusAsync(t0, "Active").ConfigureAwait(false);
        await InsertStatusAsync(t2, "Inactive").ConfigureAwait(false);
        await InsertStatusAsync(t3, "Active").ConfigureAwait(false);  // ← Duplicate entry today

        var intervals = await QueryActiveIntervalsAsync().ConfigureAwait(false);

        intervals.Should().HaveCount(2);
        intervals[0].Start.Should().Be(t0);
        intervals[0].End.Should().Be(t2);
        intervals[1].Start.Should().Be(t3);
        intervals[1].End.Should().BeNull();
    }

    [Fact(Skip = "Blocked by persons UNIQUE on value_hash — see schema-fix follow-up.")]
    public async Task ActiveIntervals_collapses_consecutive_active_observations()
    {
        // Two Active observations in a row → the LAG-based duplicate-
        // state filter in state_transitions collapses them so LEAD
        // doesn't see a spurious boundary. Expected: single interval
        // [T0, NULL).
        //
        // Note: in production today this shape doesn't naturally
        // occur — the dbt `identity_inputs_from_history` macro only
        // emits UPSERT rows on field CHANGES, so two consecutive
        // identical status values wouldn't be emitted. Test included
        // as defensive coverage in case a future writer (manual
        // operator action, parallel reconciliation) produces them.
        var t0 = new DateTime(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc);
        var t1 = new DateTime(2026, 2, 1, 0, 0, 0, DateTimeKind.Utc);
        await InsertStatusAsync(t0, "Active").ConfigureAwait(false);
        await InsertStatusAsync(t1, "Active").ConfigureAwait(false);  // ← Duplicate entry today

        var intervals = await QueryActiveIntervalsAsync().ConfigureAwait(false);

        intervals.Should().HaveCount(1);
        intervals[0].Start.Should().Be(t0);
        intervals[0].End.Should().BeNull();
    }

    [Fact]
    public async Task ActiveIntervals_returns_empty_for_person_observed_only_inactive()
    {
        // An employee whose only status observation is Inactive (e.g.
        // a long-terminated person whose Active history predates the
        // bronze snapshot) has zero active intervals. This is the
        // shape of the 433 "only-inactive" persons in the kind-cluster
        // test that resulted in 344 children with no edges at all.
        var t0 = new DateTime(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc);
        await InsertStatusAsync(t0, "Inactive").ConfigureAwait(false);

        var intervals = await QueryActiveIntervalsAsync().ConfigureAwait(false);

        intervals.Should().BeEmpty();
    }

    [Fact]
    public async Task ActiveIntervals_treats_lowercase_inactive_as_deactivation()
    {
        // The dbt model for `zoom__identity_inputs.sql` uses the
        // deactivation_condition `new_value = 'inactive'` (lower-case)
        // — the active-intervals CTE must accept both spellings.
        var t0 = new DateTime(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc);
        var t2 = new DateTime(2026, 6, 1, 0, 0, 0, DateTimeKind.Utc);
        await InsertStatusAsync(t0, "Active").ConfigureAwait(false);
        await InsertStatusAsync(t2, "inactive").ConfigureAwait(false);  // lower-case

        var intervals = await QueryActiveIntervalsAsync().ConfigureAwait(false);

        intervals.Should().HaveCount(1);
        intervals[0].End.Should().Be(t2);
    }

    // ── F4 regression: schema does NOT have UNIQUE on (child, valid_to) ──

    [Fact]
    public async Task OrgChart_schema_has_no_unique_on_valid_to()
    {
        // The migration header used to claim "UNIQUE on
        // (..., child_person_id, valid_to) enforces this invariant".
        // That UNIQUE does not exist — enforcement is via the PK plus
        // the rebuild SQL's "one current row per partition" rule. This
        // test pins the schema state so any future migration that
        // tries to add such a UNIQUE (or removes the indexes we depend
        // on) trips on the assertion.
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);

        // Collect UNIQUE/PRIMARY indexes on org_chart.
        const string sql = """
            SELECT INDEX_NAME, NON_UNIQUE
            FROM information_schema.STATISTICS
            WHERE TABLE_SCHEMA = DATABASE()
              AND TABLE_NAME   = 'org_chart'
            GROUP BY INDEX_NAME, NON_UNIQUE
            """;
        await using var cmd = new MySqlCommand(sql, conn);
        await using var reader = await cmd.ExecuteReaderAsync().ConfigureAwait(false);

        var uniqueNames = new List<string>();
        while (await reader.ReadAsync().ConfigureAwait(false))
        {
            // NON_UNIQUE=0 means UNIQUE; PRIMARY index counts as UNIQUE.
            if (reader.GetInt64("NON_UNIQUE") == 0)
                uniqueNames.Add(reader.GetString("INDEX_NAME"));
        }

        // Exactly one UNIQUE index: PRIMARY (the PK on
        // tenant, source_type, source_id, child, valid_from).
        uniqueNames.Should().BeEquivalentTo(OnlyPrimaryIndex);
    }

    // ── Helpers ────────────────────────────────────────────────────────

    private sealed record Interval(DateTime Start, DateTime? End);

    /// <summary>
    /// Inline copy of the active-intervals CTE chain from
    /// `seed-persons-from-identity-input.py` step 9 — keep in sync.
    /// Targets one person within the BambooHR source instance for the
    /// fixed `PersonId` so test assertions stay simple. Returns the
    /// (interval_start, interval_end) pairs ordered chronologically.
    /// </summary>
    private const string ActiveIntervalsCte = """
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
                created_at        AS interval_start,
                next_transition_at AS interval_end
            FROM state_transitions
            WHERE is_active = 1
        )
        SELECT interval_start, interval_end
        FROM active_intervals
        WHERE insight_tenant_id   = @tenant
          AND insight_source_type = 'bamboohr'
          AND insight_source_id   = @source
          AND person_id           = @person
        ORDER BY interval_start
        """;

    private async Task<List<Interval>> QueryActiveIntervalsAsync()
    {
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        await using var cmd = new MySqlCommand(ActiveIntervalsCte, conn);
        cmd.Parameters.AddWithValue("@tenant", TenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@source", BambooSourceId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@person", PersonId.ToByteArray(bigEndian: true));

        await using var reader = await cmd.ExecuteReaderAsync().ConfigureAwait(false);
        var result = new List<Interval>();
        while (await reader.ReadAsync().ConfigureAwait(false))
        {
            var start = reader.GetDateTime("interval_start");
            var end = await reader.IsDBNullAsync(reader.GetOrdinal("interval_end")).ConfigureAwait(false)
                ? (DateTime?)null
                : reader.GetDateTime("interval_end");
            result.Add(new Interval(start, end));
        }
        return result;
    }

    private async Task InsertStatusAsync(DateTime createdAt, string statusValue)
    {
        await using var conn = new MySqlConnection(_fixture.ConnectionString);
        await conn.OpenAsync().ConfigureAwait(false);
        const string sql = """
            INSERT INTO persons
                (value_type, insight_source_type, insight_source_id, insight_tenant_id,
                 value_id, value_full_text, value,
                 person_id, author_person_id, reason, created_at)
            VALUES
                ('status', 'bamboohr', @src, @tenant,
                 NULL, @status, NULL,
                 @person, @author, '', @ts)
            """;
        await using var cmd = new MySqlCommand(sql, conn);
        cmd.Parameters.AddWithValue("@src",    BambooSourceId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@tenant", TenantId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@status", statusValue);
        cmd.Parameters.AddWithValue("@person", PersonId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@author", AuthorPersonId.ToByteArray(bigEndian: true));
        cmd.Parameters.AddWithValue("@ts",     createdAt);
        await cmd.ExecuteNonQueryAsync().ConfigureAwait(false);
    }
}
