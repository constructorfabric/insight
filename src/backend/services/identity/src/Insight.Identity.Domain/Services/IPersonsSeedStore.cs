namespace Insight.Identity.Domain.Services;

/// <summary>
/// Persistence port for the <c>persons-seed</c> operation. Splits into
/// the resolver-feeding reads (current bindings, latest emails) and a
/// single atomic apply (observation insert + both derived-cache
/// rebuilds in one transaction). All operations are tenant-scoped.
/// </summary>
public interface IPersonsSeedStore
{
    /// <summary>
    /// Current <c>source_account_id → person_id</c> bindings in the
    /// tenant (latest <c>value_type='id'</c> per account). Feeds the
    /// known-account branch of <see cref="PersonAssignmentResolver"/>.
    /// </summary>
    Task<IReadOnlyDictionary<SourceAccountKey, Guid>> GetKnownAccountBindingsAsync(
        Guid tenantId,
        CancellationToken cancellationToken);

    /// <summary>
    /// Current email → person_id map in the tenant (latest email
    /// observation per email). Emails are returned raw — the dictionary
    /// uses a case-insensitive comparer
    /// (<see cref="StringComparer.OrdinalIgnoreCase"/>) so matching
    /// mirrors the <c>utf8mb4_unicode_ci</c> collation (ADR-0011) rather
    /// than any value normalisation. Feeds the email-link branch of
    /// <see cref="PersonAssignmentResolver"/>.
    /// </summary>
    Task<IReadOnlyDictionary<string, Guid>> GetLatestEmailToPersonAsync(
        Guid tenantId,
        CancellationToken cancellationToken);

    /// <summary>
    /// Apply the resolved seed in one transaction: INSERT IGNORE the
    /// observation rows, then tenant-scoped DELETE+INSERT rebuilds of
    /// <c>account_person_map</c> and <c>org_chart</c>. Either the whole
    /// apply commits or none of it does — a crash or cancellation
    /// mid-apply rolls back, so the tenant's caches are never left
    /// cross-inconsistent.
    /// </summary>
    Task<SeedApplyResult> ApplyAsync(
        Guid tenantId,
        IReadOnlyList<PersonObservationRow> rows,
        CancellationToken cancellationToken);
}

/// <summary>
/// Outcome of <see cref="IPersonsSeedStore.ApplyAsync"/>.
/// <see cref="ObservationsInserted"/> counts only NET-NEW rows (rows the
/// INSERT IGNORE actually wrote); duplicates suppressed by the unique
/// key are not counted, so a pure re-seed reports 0.
/// </summary>
public sealed record SeedApplyResult(int ObservationsInserted, int OrgChartEdgesRebuilt);
