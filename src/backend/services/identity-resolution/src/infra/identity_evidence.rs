use std::time::Duration;

use clickhouse::Row;
use insight_clickhouse::{Client, Config, MissingRelation};
use serde::Deserialize;
use uuid::Uuid;

use crate::domain::login_bootstrap::MAX_VALUE_ID_CHARS;
use crate::domain::review_queue::AccountDescription;
use crate::domain::seed::SourceAccountKey;

const READ_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountEvidence {
    pub account: SourceAccountKey,
    pub email: Option<String>,
    pub username: Option<String>,
    pub description: AccountDescription,
    pub is_closed: bool,
    pub states_binding_id: bool,
}

#[derive(Debug)]
pub struct EvidenceSnapshot {
    pub accounts: Vec<AccountEvidence>,
    pub truncated: bool,
}

/// Aggregates break ties on `(_synced_at, _version)`: a `ReplacingMergeTree(_version)` holds
/// duplicate rows until a merge, and `_synced_at` alone would pick between them arbitrarily;
/// the replacing version picks the row `FINAL` would keep.
const FOLD_SQL_TEMPLATE: &str = r"
    SELECT
        ifNull(insight_source_type, '')                 AS source_type,
        ifNull(toString(insight_source_id), '')         AS source_id,
        ifNull(source_account_id, '')                   AS account_id,
        argMax(ifNull(operation_type, ''), (_synced_at, _version)) AS latest_op,
        argMaxIf(
            ifNull(value, ''), (_synced_at, _version),
            value_type = 'email' AND operation_type = 'UPSERT' AND value != ''
        )                                               AS email,
        argMaxIf(
            ifNull(value, ''), (_synced_at, _version),
            value_type = 'username' AND operation_type = 'UPSERT' AND value != ''
        )                                               AS username,
        argMaxIf(
            ifNull(value, ''), (_synced_at, _version),
            value_type = 'display_name' AND operation_type = 'UPSERT' AND value != ''
        )                                               AS display_name,
        argMaxIf(
            ifNull(value, ''), (_synced_at, _version),
            value_type = 'first_name' AND operation_type = 'UPSERT' AND value != ''
        )                                               AS first_name,
        argMaxIf(
            ifNull(value, ''), (_synced_at, _version),
            value_type = 'last_name' AND operation_type = 'UPSERT' AND value != ''
        )                                               AS last_name,
        argMaxIf(
            ifNull(value, ''), (_synced_at, _version),
            value_type = 'job_title' AND operation_type = 'UPSERT' AND value != ''
        )                                               AS job_title,
        argMaxIf(
            ifNull(value, ''), (_synced_at, _version),
            value_type = 'department' AND operation_type = 'UPSERT' AND value != ''
        )                                               AS department,
        argMaxIf(
            ifNull(value, ''), (_synced_at, _version),
            value_type = 'status' AND operation_type = 'UPSERT' AND value != ''
        )                                               AS status,
        argMaxIf(
            ifNull(value, ''), (_synced_at, _version),
            value_type = 'parent_email' AND operation_type = 'UPSERT' AND value != ''
        )                                               AS manager_email,
        argMaxIf(
            ifNull(value, ''), (_synced_at, _version),
            value_type = 'id' AND operation_type = 'UPSERT' AND value != ''
            AND lengthUTF8(value) <= {MAX_ID_CHARS}
        )                                               AS binding_id
    FROM identity.identity_inputs
    WHERE source_account_id IS NOT NULL AND source_account_id != ''
    GROUP BY source_type, source_id, account_id
";

fn fold_sql() -> String {
    FOLD_SQL_TEMPLATE.replace("{MAX_ID_CHARS}", &MAX_VALUE_ID_CHARS.to_string())
}

const FOLD_COLUMNS: &str = "source_type, source_id, account_id, latest_op, email, username, \
     display_name, first_name, last_name, job_title, department, status, manager_email, \
     binding_id";

const COMPOSED_NAME_SQL: &str = "trimBoth(concatWithSeparator(' ', first_name, last_name))";

/// `lowerUTF8`, not `lower`: ClickHouse's `lower` folds ASCII only.
const ORDER_LABEL_SQL: &str = "lowerUTF8(if(email != '', email, \
     if(username != '', username, \
     if(display_name != '', display_name, \
     if({COMPOSED} != '', {COMPOSED}, account_id)))))";

const LIST_SQL: &str = r"
    SELECT {COLUMNS}, {LABEL} AS order_label
    FROM ({FOLD})
    WHERE latest_op != 'DELETE'{SOURCES}{FILTER}{RESUME}
    ORDER BY order_label, source_type, source_id, account_id
    LIMIT ?
";

const SOURCE_PREDICATE_SQL: &str = "
      AND has(?, source_type)";

const FILTER_SQL: &str = "
      AND (positionCaseInsensitiveUTF8(email, ?) > 0
           OR positionCaseInsensitiveUTF8(username, ?) > 0
           OR positionCaseInsensitiveUTF8(account_id, ?) > 0
           OR positionCaseInsensitiveUTF8(display_name, ?) > 0
           OR positionCaseInsensitiveUTF8({SOURCE_SEGMENTS}, {NEEDLE_SEGMENT}) > 0
           OR positionCaseInsensitiveUTF8({COMPOSED}, ?) > 0)";

const SOURCE_SEGMENTS_SQL: &str = "concat('_', replaceAll(source_type, '-', '_'))";

const NEEDLE_SEGMENT_SQL: &str = "concat('_', replaceAll(?, '-', '_'))";

const RESUME_SQL: &str = "
      AND ({LABEL}, source_type, source_id, account_id) > (?, ?, ?, ?)";

const MAX_EVIDENCE_ACCOUNTS: usize = 200_000;

#[derive(Debug, Row, Deserialize)]
struct FoldedRow {
    source_type: String,
    source_id: String,
    account_id: String,
    latest_op: String,
    email: String,
    username: String,
    display_name: String,
    first_name: String,
    last_name: String,
    job_title: String,
    department: String,
    status: String,
    manager_email: String,
    binding_id: String,
    order_label: String,
}

#[derive(Debug)]
pub struct ListedAccount {
    pub evidence: AccountEvidence,
    pub order_label: String,
}

#[derive(Debug, Default)]
pub struct AccountPage {
    pub accounts: Vec<ListedAccount>,
    pub more: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct AfterAccount<'a> {
    pub order_label: &'a str,
    pub source_type: &'a str,
    pub source_id: &'a str,
    pub account_id: &'a str,
}

pub struct ClickHouseEvidenceReader {
    client: Client,
}

impl ClickHouseEvidenceReader {
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    #[must_use]
    pub fn connect(url: &str, database: &str, user: &str, password: &str) -> Self {
        let mut config = Config::new(url, database).with_query_timeout(READ_TIMEOUT);
        if !user.is_empty() {
            config = config.with_auth(user, password);
        }
        Self::new(Client::new(config))
    }

    // SAFETY: tenancy shortcut (#1550): no tenant predicate, for the same reason as `identity_inputs`.
    pub async fn accounts(&self) -> anyhow::Result<EvidenceSnapshot> {
        let fold = fold_sql();
        let sql = format!(
            "SELECT {FOLD_COLUMNS}, '' AS order_label FROM ({fold}) \
             ORDER BY source_type, source_id, account_id LIMIT {MAX_EVIDENCE_ACCOUNTS}"
        );
        let rows: Vec<FoldedRow> = match self.client.query(&sql).fetch_all().await {
            Ok(rows) => rows,
            Err(e) if MissingRelation::from_error(&e).is_some() => {
                return Ok(EvidenceSnapshot {
                    accounts: Vec::new(),
                    truncated: false,
                });
            }
            Err(e) => return Err(e.into()),
        };

        let truncated = rows.len() == MAX_EVIDENCE_ACCOUNTS;
        if truncated {
            tracing::warn!(
                cap = MAX_EVIDENCE_ACCOUNTS,
                "review evidence: read cap reached; the queue and its rates \
                 describe only this many accounts, not the whole tenant"
            );
        }

        let mut accounts = Vec::with_capacity(rows.len());
        let mut skipped = 0usize;
        for row in rows {
            match map_row(row) {
                Ok(evidence) => accounts.push(evidence),
                Err(_) => skipped += 1,
            }
        }
        if skipped > 0 {
            tracing::warn!(
                skipped,
                "review evidence: rows with an unreadable source id"
            );
        }
        Ok(EvidenceSnapshot {
            accounts,
            truncated,
        })
    }
}

const EXISTS_SQL: &str = r"
    SELECT count() AS hits
    FROM identity.identity_inputs
    WHERE insight_source_type = ?
      AND toString(insight_source_id) = ?
      AND source_account_id = ?
    LIMIT 1
";

const SOURCES_SQL: &str = r"
    SELECT DISTINCT ifNull(insight_source_type, '') AS source_type
    FROM identity.identity_inputs
    WHERE source_account_id IS NOT NULL AND source_account_id != ''
      AND ifNull(insight_source_type, '') != ''
    ORDER BY source_type
";

#[derive(Debug, Row, Deserialize)]
struct SourceTypeRow {
    source_type: String,
}

#[derive(Debug, Row, Deserialize)]
struct CountRow {
    hits: u64,
}

/// `GROUP BY` is load-bearing: without it ClickHouse returns one row of default values
/// over an empty match set.
const OBSERVED_ACCOUNT_SQL: &str = r"
    SELECT
        toString(argMax(insight_source_id, _synced_at))  AS source_id,
        argMax(ifNull(operation_type, ''), _synced_at)   AS latest_op,
        argMaxIf(
            ifNull(value, ''), (_synced_at, _version),
            value_type = 'email' AND operation_type = 'UPSERT' AND value != ''
        )                                               AS email
    FROM identity.identity_inputs
    WHERE insight_source_type = ?
      AND source_account_id = ?
    GROUP BY insight_source_type, source_account_id
";

#[derive(Debug, Row, Deserialize)]
struct ObservedRow {
    source_id: String,
    latest_op: String,
    email: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedAccount {
    pub source_id: Uuid,
    pub is_closed: bool,
    pub email: Option<String>,
}

fn map_row(row: FoldedRow) -> anyhow::Result<AccountEvidence> {
    let description = AccountDescription {
        display_name: non_empty(row.display_name)
            .or_else(|| compose_name(non_empty(row.first_name), non_empty(row.last_name))),
        job_title: non_empty(row.job_title),
        department: non_empty(row.department),
        status: non_empty(row.status),
        manager_email: non_empty(row.manager_email),
    };

    Ok(AccountEvidence {
        account: SourceAccountKey {
            source_type: row.source_type,
            source_id: Uuid::parse_str(&row.source_id)?,
            account_id: row.account_id,
        },
        email: non_empty(row.email),
        username: non_empty(row.username),
        description,
        is_closed: row.latest_op == "DELETE",
        states_binding_id: !row.binding_id.trim().is_empty(),
    })
}

fn list_sql(filtered: bool, by_source: bool, resuming: bool) -> String {
    LIST_SQL
        .replace("{COLUMNS}", FOLD_COLUMNS)
        .replace("{FOLD}", &fold_sql())
        .replace(
            "{SOURCES}",
            if by_source { SOURCE_PREDICATE_SQL } else { "" },
        )
        .replace("{FILTER}", if filtered { FILTER_SQL } else { "" })
        .replace("{RESUME}", if resuming { RESUME_SQL } else { "" })
        .replace("{LABEL}", ORDER_LABEL_SQL)
        .replace("{SOURCE_SEGMENTS}", SOURCE_SEGMENTS_SQL)
        .replace("{NEEDLE_SEGMENT}", NEEDLE_SEGMENT_SQL)
        .replace("{COMPOSED}", COMPOSED_NAME_SQL)
}

fn needle_probes() -> usize {
    FILTER_SQL.matches('?').count() + NEEDLE_SEGMENT_SQL.matches('?').count()
}

fn compose_name(first: Option<String>, last: Option<String>) -> Option<String> {
    match (first, last) {
        (Some(first), Some(last)) => Some(format!("{first} {last}")),
        (Some(one), None) | (None, Some(one)) => Some(one),
        (None, None) => None,
    }
}

impl ClickHouseEvidenceReader {
    pub async fn list(
        &self,
        needle: Option<&str>,
        sources: &[String],
        after: Option<AfterAccount<'_>>,
        page_size: u64,
    ) -> anyhow::Result<AccountPage> {
        let sql = list_sql(needle.is_some(), !sources.is_empty(), after.is_some());

        let mut query = self.client.query(&sql);
        if !sources.is_empty() {
            query = query.bind(sources);
        }
        if let Some(needle) = needle {
            for _ in 0..needle_probes() {
                query = query.bind(needle);
            }
        }
        if let Some(after) = after {
            query = query
                .bind(after.order_label)
                .bind(after.source_type)
                .bind(after.source_id)
                .bind(after.account_id);
        }

        let mut rows: Vec<FoldedRow> = match query.bind(page_size + 1).fetch_all().await {
            Ok(rows) => rows,
            Err(e) if MissingRelation::from_error(&e).is_some() => {
                return Ok(AccountPage::default());
            }
            Err(e) => return Err(e.into()),
        };

        let more = rows.len() as u64 > page_size;
        rows.truncate(usize::try_from(page_size).unwrap_or(usize::MAX));

        Ok(AccountPage {
            accounts: rows
                .into_iter()
                .filter_map(|row| {
                    let order_label = row.order_label.clone();
                    map_row(row).ok().map(|evidence| ListedAccount {
                        evidence,
                        order_label,
                    })
                })
                .collect(),
            more,
        })
    }

    pub async fn has_account(&self, account: &SourceAccountKey) -> anyhow::Result<bool> {
        let row: Result<Option<CountRow>, _> = self
            .client
            .query(EXISTS_SQL)
            .bind(account.source_type.as_str())
            .bind(account.source_id.to_string())
            .bind(account.account_id.as_str())
            .fetch_optional()
            .await;

        match row {
            Ok(hit) => Ok(hit.is_some_and(|r| r.hits > 0)),
            Err(e) if MissingRelation::from_error(&e).is_some() => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    pub async fn sources(&self) -> anyhow::Result<Vec<String>> {
        let rows: Vec<SourceTypeRow> = match self.client.query(SOURCES_SQL).fetch_all().await {
            Ok(rows) => rows,
            Err(e) if MissingRelation::from_error(&e).is_some() => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };

        Ok(rows.into_iter().map(|row| row.source_type).collect())
    }

    pub async fn observed_account(
        &self,
        source_type: &str,
        account_id: &str,
    ) -> anyhow::Result<Option<ObservedAccount>> {
        let row: Result<Option<ObservedRow>, _> = self
            .client
            .query(OBSERVED_ACCOUNT_SQL)
            .bind(source_type)
            .bind(account_id)
            .fetch_optional()
            .await;

        let found = match row {
            Ok(found) => found,
            Err(e) if MissingRelation::from_error(&e).is_some() => None,
            Err(e) => return Err(e.into()),
        };

        let Some(row) = found.filter(|r| !r.source_id.trim().is_empty()) else {
            return Ok(None);
        };

        let source_id = Uuid::parse_str(row.source_id.trim())?;
        if source_id.is_nil() {
            return Ok(None);
        }

        Ok(Some(ObservedAccount {
            source_id,
            is_closed: row.latest_op == "DELETE",
            email: non_empty(row.email),
        }))
    }
}

fn non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fold_reads_a_stated_id_as_bindable_and_a_missing_one_as_not() -> anyhow::Result<()> {
        assert!(map_row(folded("UPSERT", "sam@corp.com", "sam"))?.states_binding_id);

        let mut silent = folded("UPSERT", "sam@corp.com", "sam");
        silent.binding_id = String::new();
        assert!(
            !map_row(silent)?.states_binding_id,
            "an account no connector states an id for is not bindable"
        );
        Ok(())
    }

    #[test]
    fn the_fold_caps_the_id_at_the_seed_s_limit() {
        assert!(
            fold_sql().contains(&format!("lengthUTF8(value) <= {MAX_VALUE_ID_CHARS}")),
            "the fold must carry the seed's cap, not a second opinion"
        );
    }

    fn folded(latest_op: &str, email: &str, username: &str) -> FoldedRow {
        FoldedRow {
            source_type: "github".to_owned(),
            source_id: Uuid::from_u128(1).to_string(),
            account_id: "gh-1".to_owned(),
            latest_op: latest_op.to_owned(),
            email: email.to_owned(),
            username: username.to_owned(),
            display_name: String::new(),
            first_name: String::new(),
            last_name: String::new(),
            job_title: String::new(),
            department: String::new(),
            status: String::new(),
            manager_email: String::new(),
            binding_id: "gh-1".to_owned(),
            order_label: String::new(),
        }
    }

    #[test]
    fn closure_is_read_from_the_latest_operation() -> anyhow::Result<()> {
        assert!(map_row(folded("DELETE", "", ""))?.is_closed);
        assert!(!map_row(folded("UPSERT", "a@example.com", ""))?.is_closed);
        Ok(())
    }

    #[test]
    fn blank_values_are_absent_not_empty_strings() -> anyhow::Result<()> {
        let evidence = map_row(folded("UPSERT", "  ", ""))?;
        assert_eq!(evidence.email, None, "whitespace is not evidence");
        assert_eq!(evidence.username, None);
        Ok(())
    }

    #[test]
    fn a_username_only_account_keeps_its_value() -> anyhow::Result<()> {
        let evidence = map_row(folded("UPSERT", "", "octocat"))?;
        assert_eq!(evidence.username.as_deref(), Some("octocat"));
        assert_eq!(evidence.email, None);
        Ok(())
    }

    #[test]
    fn an_account_with_no_matchable_value_still_arrives_described() -> anyhow::Result<()> {
        let mut row = folded("UPSERT", "", "");
        row.display_name = "Ann Lee".to_owned();
        row.job_title = "Engineer".to_owned();
        row.department = "Platform".to_owned();
        row.status = "Active".to_owned();
        row.manager_email = "lead@example.com".to_owned();

        let evidence = map_row(row)?;

        assert_eq!(evidence.email, None, "nothing to match on");
        assert_eq!(
            evidence.description.display_name.as_deref(),
            Some("Ann Lee")
        );
        assert_eq!(evidence.description.job_title.as_deref(), Some("Engineer"));
        assert_eq!(evidence.description.department.as_deref(), Some("Platform"));
        assert_eq!(evidence.description.status.as_deref(), Some("Active"));
        assert_eq!(
            evidence.description.manager_email.as_deref(),
            Some("lead@example.com")
        );
        Ok(())
    }

    #[test]
    fn a_name_sent_in_parts_is_composed() -> anyhow::Result<()> {
        let mut parts = folded("UPSERT", "", "");
        parts.first_name = "Ann".to_owned();
        parts.last_name = "Lee".to_owned();
        assert_eq!(
            map_row(parts)?.description.display_name.as_deref(),
            Some("Ann Lee")
        );

        let mut both = folded("UPSERT", "", "");
        both.display_name = "A. Lee".to_owned();
        both.first_name = "Ann".to_owned();
        both.last_name = "Lee".to_owned();
        assert_eq!(
            map_row(both)?.description.display_name.as_deref(),
            Some("A. Lee"),
            "an observed whole name wins over one assembled from parts"
        );

        let mut half = folded("UPSERT", "", "");
        half.last_name = "Lee".to_owned();
        assert_eq!(
            map_row(half)?.description.display_name.as_deref(),
            Some("Lee")
        );
        Ok(())
    }

    #[test]
    fn every_placeholder_of_the_listing_has_a_value_bound_to_it() {
        for (case, filtered, by_source, resuming, expected) in [
            ("browse", false, false, false, 1),
            ("search", true, false, false, 1 + needle_probes()),
            ("browse, resumed", false, false, true, 5),
            ("search, resumed", true, false, true, 5 + needle_probes()),
            ("browse one source", false, true, false, 2),
            ("search one source", true, true, false, 2 + needle_probes()),
            ("everything on", true, true, true, 6 + needle_probes()),
        ] {
            assert_eq!(
                list_sql(filtered, by_source, resuming).matches('?').count(),
                expected,
                "placeholder count changed for: {case}"
            );
        }
    }

    #[test]
    fn an_unfiltered_listing_carries_no_source_predicate() {
        let sql = list_sql(true, false, true);

        assert!(
            !sql.contains(SOURCE_PREDICATE_SQL.trim()),
            "unexpected source predicate in: {sql}"
        );
    }

    #[test]
    fn the_source_predicate_narrows_the_fold_output_not_the_fold() {
        let sql = list_sql(false, true, false);

        let fold = fold_sql();
        let fold_end = sql.find(&fold).map(|at| at + fold.len());
        let predicate = sql.find(SOURCE_PREDICATE_SQL.trim());

        assert!(
            fold_end.is_some() && predicate > fold_end,
            "the predicate must sit after the fold's closing: {sql}"
        );
        assert_eq!(
            sql.matches(SOURCE_PREDICATE_SQL.trim()).count(),
            1,
            "exactly one source predicate: {sql}"
        );
    }

    #[test]
    fn the_sources_read_is_distinct_ordered_and_skips_empty_account_ids() {
        let sql = SOURCES_SQL.split_whitespace().collect::<Vec<_>>().join(" ");

        for needle in [
            "SELECT DISTINCT",
            "FROM identity.identity_inputs",
            "source_account_id IS NOT NULL AND source_account_id != ''",
            "ORDER BY source_type",
        ] {
            assert!(sql.contains(needle), "missing `{needle}` in: {sql}");
        }
    }

    #[test]
    fn a_needle_and_sources_are_both_joined_with_and() {
        let sql = list_sql(true, true, false);
        let normalized = sql.split_whitespace().collect::<Vec<_>>().join(" ");

        assert!(
            normalized
                .contains("WHERE latest_op != 'DELETE' AND has(?, source_type) AND (positionCase"),
            "the source predicate and the needle filter must each be ANDed: {sql}"
        );
        assert!(
            sql.find(SOURCE_PREDICATE_SQL.trim()) < sql.find("AND (positionCase"),
            "the needle filter must follow the source predicate: {sql}"
        );
    }

    #[test]
    fn a_needle_is_probed_against_every_value_a_row_can_show() {
        let sql = list_sql(true, false, false);

        assert_eq!(
            needle_probes(),
            6,
            "one probe per displayable value: address, handle, id, name, source, \
             composed name"
        );
        assert_eq!(
            sql.matches("positionCaseInsensitiveUTF8").count(),
            needle_probes(),
            "a probe without its placeholder, or the other way round"
        );
        assert!(
            sql.contains(&format!(
                "positionCaseInsensitiveUTF8({COMPOSED_NAME_SQL}, ?)"
            )),
            "the composed name is not searched: {sql}"
        );
        assert!(
            sql.contains(&format!(
                "positionCaseInsensitiveUTF8({SOURCE_SEGMENTS_SQL}, {NEEDLE_SEGMENT_SQL})"
            )),
            "the source is not probed by segment: {sql}"
        );
    }

    #[test]
    fn the_listing_leaves_no_template_hole_unfilled() {
        let sql = list_sql(true, true, true);

        assert!(!sql.contains('{'), "unsubstituted hole in: {sql}");
        assert_eq!(
            sql.matches("lowerUTF8").count(),
            2,
            "the order key and the resume comparison must be the same expression"
        );
    }

    #[test]
    fn the_order_key_falls_back_the_way_the_row_reads() {
        let sql = list_sql(false, false, false);

        for value in [
            "email",
            "username",
            "display_name",
            "first_name",
            "account_id",
        ] {
            assert!(sql.contains(value), "the order key ignores {value}");
        }
    }
}
