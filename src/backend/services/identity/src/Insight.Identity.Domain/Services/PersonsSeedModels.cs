namespace Insight.Identity.Domain.Services;

/// <summary>
/// Identifies one source-native account within a tenant. The seed
/// operation is tenant-scoped, so the tenant is implied by the run and
/// not part of the key.
/// </summary>
public readonly record struct SourceAccountKey(
    string InsightSourceType,
    Guid InsightSourceId,
    string SourceAccountId);

/// <summary>
/// One source-account with the observations the seed run saw for it,
/// plus the two facts the resolver needs: its current email and
/// whether the account is currently closed (latest observation was a
/// DELETE). <see cref="Observations"/> holds only the UPSERT rows
/// (the values to write to <c>persons</c>); DELETE rows are collapsed
/// into <see cref="IsClosed"/>.
/// </summary>
public sealed record SeedProfile(
    SourceAccountKey Account,
    IReadOnlyList<IdentityInputRow> Observations,
    string? LatestEmail,
    bool IsClosed);

/// <summary>
/// A set of profiles a resolver claims belong to the same person.
/// (Future resolvers — graph, similarity — emit the same shape.)
/// </summary>
public sealed record ProfileGroup(IReadOnlyList<SeedProfile> Profiles);

/// <summary>How a profile group was bound to its <c>person_id</c>.</summary>
public enum AssignmentKind
{
    /// <summary>At least one profile already had a current account binding; reused it.</summary>
    ReusedKnown,
    /// <summary>The group's email matched an existing person; linked (reason <c>auto-seed-link</c>).</summary>
    LinkedByEmail,
    /// <summary>No binding, no email match, at least one active profile; minted a new person.</summary>
    Minted,
}

/// <summary>
/// Resolution outcome for one profile group: the <c>person_id</c> every
/// profile in the group binds to, and how that binding was decided.
/// </summary>
public sealed record PersonAssignment(
    Guid PersonId,
    AssignmentKind Kind,
    IReadOnlyList<SeedProfile> Profiles);

/// <summary>
/// One row to INSERT into <c>persons</c>. Exactly one of
/// <see cref="ValueId"/> / <see cref="ValueFullText"/> /
/// <see cref="Value"/> is non-null (the value-routing rule decides
/// which). <see cref="CreatedAt"/> is the source <c>_synced_at</c>,
/// preserving chronological observation history.
/// </summary>
public sealed record PersonObservationRow(
    string ValueType,
    string InsightSourceType,
    Guid InsightSourceId,
    Guid InsightTenantId,
    string? ValueId,
    string? ValueFullText,
    string? Value,
    Guid PersonId,
    Guid AuthorPersonId,
    string Reason,
    DateTime CreatedAt);

/// <summary>
/// Counters returned by a completed <c>persons-seed</c> run, serialised
/// into <c>operations.summary_json</c>. The <c>Accounts*</c> counters
/// are per-account (a profile in a group counts once), not per-person.
/// <see cref="ObservationsInserted"/> is NET-NEW only — rows the
/// INSERT IGNORE actually wrote; duplicates already present are not
/// counted, so a pure re-seed reports 0 even though every account was
/// processed.
/// </summary>
public sealed record PersonsSeedSummary(
    int AccountsRead,
    int AccountsReusedKnown,
    int AccountsLinkedByEmail,
    int AccountsMintedNew,
    int AccountsSkippedClosed,
    int AccountsSkippedNoEmail,
    int ObservationsInserted,
    int OrgChartEdgesRebuilt);
