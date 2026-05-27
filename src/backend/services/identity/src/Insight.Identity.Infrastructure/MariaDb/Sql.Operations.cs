namespace Insight.Identity.Infrastructure.MariaDb;

/// <summary>
/// SQL for the generic <c>operations</c> audit table (migration 011).
/// Every query filters by <c>insight_tenant_id</c> except the lifecycle
/// transitions (<c>TryStart</c>/<c>Complete</c>/<c>Fail</c>) which are
/// keyed by the immutable <c>operation_id</c> primary key — the
/// originating POST handler verified tenant ownership when enqueuing.
/// </summary>
internal static class SqlOperations
{
    private const string ColumnList =
        "operation_id, operation_type, status, insight_tenant_id, author_person_id, " +
        "request_json, summary_json, error_message, started_at, completed_at";

    public const string Insert = """
        INSERT INTO operations
            (operation_id, operation_type, status,
             insight_tenant_id, author_person_id,
             request_json)
        VALUES
            (@operation_id, @operation_type, 'queued',
             @tenant_id, @author_person_id,
             @request_json)
        """;

    /// <summary>
    /// Flip <c>queued</c> → <c>running</c> only. <c>rows_affected = 0</c>
    /// when the row was already picked up — caller treats that as a
    /// no-op so two workers cannot run the same operation twice.
    /// </summary>
    public const string TryStart = """
        UPDATE operations
        SET status = 'running'
        WHERE operation_id = @operation_id
          AND status       = 'queued'
        """;

    public const string Complete = """
        UPDATE operations
        SET status       = 'completed',
            summary_json = @summary_json,
            completed_at = UTC_TIMESTAMP(6)
        WHERE operation_id = @operation_id
        """;

    public const string Fail = """
        UPDATE operations
        SET status        = 'failed',
            error_message = @error_message,
            completed_at  = UTC_TIMESTAMP(6)
        WHERE operation_id = @operation_id
        """;

    public const string GetById = $"""
        SELECT {ColumnList}
        FROM operations
        WHERE insight_tenant_id = @tenant_id
          AND operation_id      = @operation_id
        LIMIT 1
        """;

    public const string ListBase = $"""
        SELECT {ColumnList}
        FROM operations
        WHERE insight_tenant_id = @tenant_id
        """;

    /// <summary>
    /// Mark all <c>queued</c>/<c>running</c> rows older than the cutoff
    /// as failed. Run once at service startup so a pod restart cannot
    /// leave a row stuck in <c>running</c> forever. Intentionally NOT
    /// tenant-scoped (unlike every other query here): the single-process
    /// worker owns all in-flight operations across tenants, so the
    /// startup sweep reclaims every orphan regardless of tenant.
    /// </summary>
    public const string SweepZombies = """
        UPDATE operations
        SET status        = 'failed',
            error_message = 'aborted by pod restart',
            completed_at  = UTC_TIMESTAMP(6)
        WHERE status IN ('queued', 'running')
          AND started_at < @older_than
        """;
}
