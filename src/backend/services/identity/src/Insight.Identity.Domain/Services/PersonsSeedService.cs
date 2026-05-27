namespace Insight.Identity.Domain.Services;

/// <summary>
/// Orchestrates a <c>persons-seed</c> run for one tenant. Three phases
/// mirror the IR resolver vision so the mechanism extends to the full
/// admin-review flow later without restructuring:
/// <list type="number">
///   <item><b>Build + resolve (suggest).</b> Stream <c>identity_inputs</c>,
///   fold into per-account <see cref="SeedProfile"/>s, group by email
///   (<see cref="EmailProfileResolver"/>).</item>
///   <item><b>Assign.</b> Map each group to a <c>person_id</c>
///   (<see cref="PersonAssignmentResolver"/>): reuse known binding,
///   else link by email, else mint (active only).</item>
///   <item><b>Apply.</b> INSERT IGNORE the observations, then rebuild
///   <c>account_person_map</c> and <c>org_chart</c> tenant-scoped.</item>
/// </list>
/// The email collision rule differs from the legacy Python script: an
/// unknown account whose email matches an existing person is LINKED
/// (reason <c>auto-seed-link</c>), not flagged pending-iresolution.
/// </summary>
public sealed class PersonsSeedService
{
    private readonly IIdentityInputsReader _inputs;
    private readonly IPersonsSeedStore _store;
    private readonly Func<Guid> _mintPersonId;

    public PersonsSeedService(IIdentityInputsReader inputs, IPersonsSeedStore store)
        : this(inputs, store, Guid.NewGuid)
    {
    }

    // Test-seam ctor: inject a deterministic person-id factory.
    internal PersonsSeedService(IIdentityInputsReader inputs, IPersonsSeedStore store, Func<Guid> mintPersonId)
    {
        _inputs = inputs;
        _store = store;
        _mintPersonId = mintPersonId;
    }

    public async Task<PersonsSeedSummary> RunAsync(
        Guid tenantId,
        Guid authorPersonId,
        CancellationToken cancellationToken)
    {
        // ── Phase 1: build per-account profiles from the input stream ──
        var profiles = await BuildProfilesAsync(tenantId, cancellationToken).ConfigureAwait(false);
        var accountsRead = profiles.Count;

        // ── Phase 1 (resolver) + Phase 2 (assign) ─────────────────────
        var groups = EmailProfileResolver.Group(profiles);
        var known = await _store.GetKnownAccountBindingsAsync(tenantId, cancellationToken).ConfigureAwait(false);
        var emailToPerson = await _store.GetLatestEmailToPersonAsync(tenantId, cancellationToken).ConfigureAwait(false);
        var resolved = PersonAssignmentResolver.Resolve(groups, known, emailToPerson, _mintPersonId);

        // ── Phase 3: apply (one transaction) ───────────────────────────
        var rows = BuildObservationRows(resolved.Assignments, tenantId, authorPersonId);
        var applied = await _store.ApplyAsync(tenantId, rows, cancellationToken).ConfigureAwait(false);

        return new PersonsSeedSummary(
            AccountsRead:          accountsRead,
            AccountsReusedKnown:   resolved.ReusedKnown,
            AccountsLinkedByEmail: resolved.LinkedByEmail,
            AccountsMintedNew:     resolved.MintedNew,
            AccountsSkippedClosed: resolved.SkippedClosed,
            AccountsSkippedNoEmail: resolved.SkippedNoEmail,
            ObservationsInserted:  applied.ObservationsInserted,
            OrgChartEdgesRebuilt:  applied.OrgChartEdgesRebuilt);
    }

    private async Task<IReadOnlyList<SeedProfile>> BuildProfilesAsync(
        Guid tenantId,
        CancellationToken cancellationToken)
    {
        // Rows arrive ordered by account then _synced_at DESC, so the
        // first row of each account is its latest observation. We fold
        // each account's rows into one profile, collecting UPSERT rows
        // for the apply step and deriving (latest email, isClosed) from
        // the latest-first ordering.
        var byAccount = new Dictionary<SourceAccountKey, AccountAccumulator>();

        await foreach (var row in _inputs.StreamAsync(tenantId, cancellationToken).ConfigureAwait(false))
        {
            var key = new SourceAccountKey(row.InsightSourceType, row.InsightSourceId, row.SourceAccountId);
            if (!byAccount.TryGetValue(key, out var acc))
            {
                acc = new AccountAccumulator();
                byAccount[key] = acc;
            }
            acc.Add(row);
        }

        var profiles = new List<SeedProfile>(byAccount.Count);
        foreach (var (key, acc) in byAccount)
        {
            profiles.Add(new SeedProfile(
                Account: key,
                Observations: acc.Upserts,
                LatestEmail: acc.LatestEmail,
                IsClosed: acc.IsClosed));
        }
        return profiles;
    }

    private static List<PersonObservationRow> BuildObservationRows(
        IReadOnlyList<PersonAssignment> assignments,
        Guid tenantId,
        Guid authorPersonId)
    {
        var rows = new List<PersonObservationRow>();
        foreach (var assignment in assignments)
        {
            // Only email-linked rows carry the auto-seed-link reason for
            // forensic traceability. Reused-known and freshly-minted rows
            // get a blank reason, matching the legacy seeder's blank
            // reason for non-pending writes.
            var reason = assignment.Kind == AssignmentKind.LinkedByEmail
                ? PersonAssignmentResolver.AutoSeedLinkReason
                : string.Empty;

            foreach (var profile in assignment.Profiles)
            {
                foreach (var obs in profile.Observations)
                {
                    var (valueId, valueFullText, value) = ValueRouting.Route(obs.ValueType, obs.Value);
                    if (valueId is null && valueFullText is null && value is null)
                    {
                        continue; // oversized — dropped per routing rule
                    }
                    rows.Add(new PersonObservationRow(
                        ValueType:         obs.ValueType,
                        InsightSourceType: obs.InsightSourceType,
                        InsightSourceId:   obs.InsightSourceId,
                        InsightTenantId:   tenantId,
                        ValueId:           valueId,
                        ValueFullText:     valueFullText,
                        Value:             value,
                        PersonId:          assignment.PersonId,
                        AuthorPersonId:    authorPersonId,
                        Reason:            reason,
                        CreatedAt:         obs.SyncedAt));
                }
            }
        }
        return rows;
    }

    /// <summary>
    /// Folds one account's input rows (delivered latest-first) into the
    /// facts the seed needs. The first row seen for any value_type is
    /// that type's latest value; the first email row's value is the
    /// account's current email; the very first row's DELETE flag marks
    /// the account closed.
    /// </summary>
    private sealed class AccountAccumulator
    {
        private bool _sawAny;

        public List<IdentityInputRow> Upserts { get; } = new();
        public string? LatestEmail { get; private set; }
        public bool IsClosed { get; private set; }

        public void Add(IdentityInputRow row)
        {
            if (!_sawAny)
            {
                // First row overall = latest observation on the account.
                IsClosed = row.IsDelete;
                _sawAny = true;
            }
            if (string.Equals(row.ValueType, "email", StringComparison.Ordinal)
                && LatestEmail is null
                && !string.IsNullOrWhiteSpace(row.Value))
            {
                // Store the email as-is (no lower/trim). Case-insensitive
                // matching is the comparer's job downstream
                // (StringComparer.OrdinalIgnoreCase in the resolvers,
                // utf8mb4_unicode_ci collation in SQL) — see ADR-0011.
                LatestEmail = row.Value;
            }
            // DELETE rows are signal only — they never become persons rows.
            if (!row.IsDelete)
            {
                Upserts.Add(row);
            }
        }
    }
}
