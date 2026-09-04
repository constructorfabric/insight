use std::sync::LazyLock;

use super::model::{Gear, GearRow};

const STATUS_FIELD: &str = "Status";
const DESIGN_FIELD: &str = "Design";
const SDK_FIELD: &str = "SDK";
const COMMITMENT_FIELD: &str = "Commitment";
const PRIORITY_FIELD: &str = "Prio (A.C.V.Ag)";
const EFFORT_FIELD: &str = "Estimated Efforts m*d";

/// Cards one board will serve. A delivery board is curated by hand and nothing
/// like this size in practice, so reaching it means the board stopped being one
/// — the read logs when it truncates rather than quietly serving a partial one.
const GEAR_LIMIT: usize = 2_000;

// SAFETY: every interpolated name below is a module constant, never caller input.
static READ_GEARS_SQL: LazyLock<String> = LazyLock::new(|| {
    let status = single_select(STATUS_FIELD);
    let design = single_select(DESIGN_FIELD);
    let sdk = single_select(SDK_FIELD);
    let commitment = single_select(COMMITMENT_FIELD);
    let priority = text_value(PRIORITY_FIELD);
    let effort = issue_number_value(EFFORT_FIELD);

    format!(
        "WITH board_items AS (
            SELECT
                content_repo_full_name AS repo_full_name,
                content_number AS content_number,
                argMax(ifNull(field_values_json, '[]'), ifNull(updated_at, '')) AS field_values,
                argMax(ifNull(source_id, ''), ifNull(updated_at, '')) AS source_id,
                argMax(ifNull(is_archived, false), ifNull(updated_at, '')) AS archived
            FROM bronze_github.project_items
            WHERE project_number = ?
              AND content_number > 0
            GROUP BY repo_full_name, content_number
            HAVING NOT archived
        ),
        board_issues AS (
            SELECT
                repo_full_name AS repo_full_name,
                number AS number,
                argMax(ifNull(title, ''), ifNull(collected_at, '')) AS title,
                argMax(ifNull(state, ''), ifNull(collected_at, '')) AS state,
                argMax(ifNull(milestone_title, ''), ifNull(collected_at, '')) AS milestone_title,
                argMax(ifNull(assignee_logins, '[]'), ifNull(collected_at, '')) AS assignee_logins,
                argMax(ifNull(issue_field_values_json, '[]'), ifNull(collected_at, '')) AS issue_fields
            FROM bronze_github.issues
            WHERE repo_full_name IN (SELECT repo_full_name FROM board_items)
            GROUP BY repo_full_name, number
        )
        SELECT
            toInt64(assumeNotNull(board_items.content_number)) AS number,
            board_issues.title AS title,
            {status} AS status,
            {design} AS design,
            {sdk} AS sdk,
            {commitment} AS commitment,
            {priority} AS priority,
            {effort} AS effort_man_days,
            board_issues.milestone_title AS milestone_title,
            JSONExtract(board_issues.assignee_logins, 'Array(String)') AS assignees,
            board_issues.state = 'closed' AS closed,
            ifNull(board_issues.repo_full_name, '') AS repo_full_name,
            board_items.source_id AS source_id
        FROM board_items
        INNER JOIN board_issues
            ON board_issues.repo_full_name = assumeNotNull(board_items.repo_full_name)
           AND board_issues.number = board_items.content_number
        ORDER BY title
        LIMIT {GEAR_LIMIT}"
    )
});

fn single_select(field: &str) -> String {
    format!("JSONExtractString({}, 'name')", field_value(field))
}

fn text_value(field: &str) -> String {
    format!("JSONExtractString({}, 'text')", field_value(field))
}

fn field_value(field: &str) -> String {
    format!(
        "arrayFirst(value -> JSONExtractString(JSONExtractRaw(value, 'field'), 'name') = '{field}', \
         JSONExtractArrayRaw(board_items.field_values))"
    )
}

fn issue_number_value(field: &str) -> String {
    format!(
        "JSONExtractFloat(arrayFirst(value -> JSONExtractString(value, 'issue_field_name') = '{field}', \
         JSONExtractArrayRaw(board_issues.issue_fields)), 'value')"
    )
}

pub(crate) async fn read_gears(
    ch: &insight_clickhouse::Client,
    project_number: i64,
) -> Result<Vec<Gear>, clickhouse::error::Error> {
    let rows = ch
        .query(&READ_GEARS_SQL)
        .bind(project_number)
        .fetch_all::<GearRow>()
        .await?;

    if rows.len() >= GEAR_LIMIT {
        tracing::warn!(
            project_number,
            limit = GEAR_LIMIT,
            "gear roadmap truncated at the card limit"
        );
    }

    Ok(rows.into_iter().map(Gear::from_row).collect())
}

#[cfg(test)]
mod tests {
    use super::{GEAR_LIMIT, READ_GEARS_SQL};

    #[test]
    fn the_statement_reads_one_board_and_binds_its_number() {
        assert!(READ_GEARS_SQL.contains("WHERE project_number = ?"));
        assert_eq!(READ_GEARS_SQL.matches('?').count(), 1);
    }

    /// `GearRow` reads every column into a non-optional field, and the row
    /// decoder rejects `Nullable(String)` for a `String`. The CTEs default
    /// their aggregates, but a column carried through as a GROUP BY key keeps
    /// the source nullability and has to be defaulted in the projection.
    #[test]
    fn the_repository_is_projected_with_a_default() {
        assert!(READ_GEARS_SQL.contains("ifNull(board_issues.repo_full_name, '')"));
    }

    #[test]
    fn archived_items_and_draft_cards_stay_out() {
        assert!(READ_GEARS_SQL.contains("HAVING NOT archived"));
        assert!(READ_GEARS_SQL.contains("content_number > 0"));
    }

    /// `project_items` is a `ReplacingMergeTree` keyed per card, so an archived
    /// re-read and its older active version both sit there until a merge.
    /// Filtering rows before `argMax` drops the archived latest and groups the
    /// stale active one, so the card comes back from the dead.
    #[test]
    fn the_archive_flag_is_resolved_before_it_is_filtered() {
        assert!(READ_GEARS_SQL.contains("argMax(ifNull(is_archived, false)"));
        assert!(!READ_GEARS_SQL.contains("AND NOT ifNull(is_archived, false)"));
    }

    #[test]
    fn the_issue_read_is_narrowed_to_the_repositories_on_the_board() {
        assert!(
            READ_GEARS_SQL.contains("repo_full_name IN (SELECT repo_full_name FROM board_items)")
        );
    }

    #[test]
    fn the_read_is_bounded() {
        assert!(READ_GEARS_SQL.contains(&format!("LIMIT {GEAR_LIMIT}")));
    }
}
