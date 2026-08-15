//! Folded connector evidence for the review surface (ClickHouse).
//!
//! The review queue is derived from two sources joined on the account key: this
//! evidence — every account a connector has observed, including e-mail-less
//! ones — and the current bindings in `persons`. Folding happens in ClickHouse:
//! one row per account carrying its latest values and whether its latest event
//! closed it (a DELETE tombstone), so closed accounts drop out of the queue.

use std::time::Duration;

use clickhouse::Row;
use insight_clickhouse::{Client, Config};
use serde::Deserialize;
use uuid::Uuid;

use crate::domain::review_queue::AccountDescription;
use crate::domain::seed::SourceAccountKey;

const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// One account as the evidence currently describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountEvidence {
    pub account: SourceAccountKey,
    pub email: Option<String>,
    pub username: Option<String>,
    /// How the source describes the account — for a reader, not the matcher.
    pub description: AccountDescription,
    /// The account's latest event is a closure signal — it is deactivated at
    /// the source and must not be surfaced for review.
    pub is_closed: bool,
}

/// The whole evidence read, with the one fact about the read itself a consumer
/// must not lose: whether the cap cut it short. Rates derived from a truncated
/// read describe a prefix of the tenant, and only the caller can say so to the
/// operator.
#[derive(Debug)]
pub struct EvidenceSnapshot {
    pub accounts: Vec<AccountEvidence>,
    pub truncated: bool,
}

/// One row per observed account: latest operation and latest non-empty values.
/// DELETE rows carry an empty value by contract, so the value aggregates filter
/// on UPSERT — while the operation aggregate keeps every event, which is what
/// makes closure detectable.
///
/// The descriptive attributes are read for the operator, not for the matcher:
/// an account with no address is bound by a human or by nobody, and a human
/// needs to recognise whose it is. Same scan, same grouping.
const FOLD_SQL: &str = r"
    SELECT
        ifNull(insight_source_type, '')                 AS source_type,
        ifNull(toString(insight_source_id), '')         AS source_id,
        ifNull(source_account_id, '')                   AS account_id,
        argMax(ifNull(operation_type, ''), _synced_at)  AS latest_op,
        argMaxIf(
            ifNull(value, ''), _synced_at,
            value_type = 'email' AND operation_type = 'UPSERT' AND value != ''
        )                                               AS email,
        argMaxIf(
            ifNull(value, ''), _synced_at,
            value_type = 'username' AND operation_type = 'UPSERT' AND value != ''
        )                                               AS username,
        argMaxIf(
            ifNull(value, ''), _synced_at,
            value_type = 'display_name' AND operation_type = 'UPSERT' AND value != ''
        )                                               AS display_name,
        argMaxIf(
            ifNull(value, ''), _synced_at,
            value_type = 'first_name' AND operation_type = 'UPSERT' AND value != ''
        )                                               AS first_name,
        argMaxIf(
            ifNull(value, ''), _synced_at,
            value_type = 'last_name' AND operation_type = 'UPSERT' AND value != ''
        )                                               AS last_name,
        argMaxIf(
            ifNull(value, ''), _synced_at,
            value_type = 'job_title' AND operation_type = 'UPSERT' AND value != ''
        )                                               AS job_title,
        argMaxIf(
            ifNull(value, ''), _synced_at,
            value_type = 'department' AND operation_type = 'UPSERT' AND value != ''
        )                                               AS department,
        argMaxIf(
            ifNull(value, ''), _synced_at,
            value_type = 'status' AND operation_type = 'UPSERT' AND value != ''
        )                                               AS status,
        argMaxIf(
            ifNull(value, ''), _synced_at,
            value_type = 'parent_email' AND operation_type = 'UPSERT' AND value != ''
        )                                               AS manager_email
    FROM identity.identity_inputs
    WHERE source_account_id IS NOT NULL AND source_account_id != ''
    GROUP BY source_type, source_id, account_id
    ORDER BY source_type, source_id, account_id
";

/// Accounts whose current values contain the needle, for the operator who has
/// an account in hand and no idea whose it is. The fold is the same one the
/// queue reads; only the filter and the ceiling differ.
const SEARCH_SQL: &str = r"
    SELECT source_type, source_id, account_id, latest_op, email, username,
           display_name, first_name, last_name, job_title, department, status,
           manager_email
    FROM ({FOLD})
    WHERE latest_op != 'DELETE'
      AND (positionCaseInsensitive(email, ?) > 0
           OR positionCaseInsensitive(username, ?) > 0
           OR positionCaseInsensitive(account_id, ?) > 0
           OR positionCaseInsensitive(display_name, ?) > 0)
    LIMIT ?
";

/// Ceiling on the fold, which is read whole into memory. The queue's rates are
/// meant to cover every observed account, so this is a safety valve against an
/// unbounded read rather than a pagination knob: reaching it truncates what the
/// operator sees, and is reported rather than passed off as a complete answer.
/// The fold is ordered so a truncated read is at least the same prefix twice.
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
}

/// Reads folded evidence for the review surface.
pub struct ClickHouseEvidenceReader {
    client: Client,
}

impl ClickHouseEvidenceReader {
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Build from connection settings (empty user → no auth), like the seed's
    /// input reader.
    #[must_use]
    pub fn connect(url: &str, database: &str, user: &str, password: &str) -> Self {
        let mut config = Config::new(url, database).with_query_timeout(READ_TIMEOUT);
        if !user.is_empty() {
            config = config.with_auth(user, password);
        }
        Self::new(Client::new(config))
    }

    /// Every observed account with its folded evidence.
    ///
    /// The tenant is not a predicate here for the same reason the seed's reader
    /// drops it: the evidence carries a producer-side hashed tenant that never
    /// equals the caller's. Single-tenant deployments only — restoring the
    /// filter is a multi-tenant prerequisite (see `identity_inputs`).
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails or a stored source id is not a UUID.
    pub async fn accounts(&self) -> anyhow::Result<EvidenceSnapshot> {
        let sql = format!("{FOLD_SQL} LIMIT {MAX_EVIDENCE_ACCOUNTS}");
        let rows: Vec<FoldedRow> = match self.client.query(&sql).fetch_all().await {
            Ok(rows) => rows,
            Err(e) if is_missing_relation(&e) => {
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

        // A row whose source id is not a UUID is one unusable account, not a
        // reason to blind the whole review surface: skip it and say so.
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

/// Whether a connector has ever observed this account. Asked before the verbs
/// that only make sense for an account that exists — an operator may
/// pre-register a binding, but detaching or excluding something nothing has
/// seen would mint a person for a typo.
const EXISTS_SQL: &str = r"
    SELECT count() AS hits
    FROM identity.identity_inputs
    WHERE insight_source_type = ?
      AND toString(insight_source_id) = ?
      AND source_account_id = ?
    LIMIT 1
";

#[derive(Debug, Row, Deserialize)]
struct CountRow {
    hits: u64,
}

/// The account as the connectors last described it, for the login bootstrap:
/// which instance carries it, whether the source has closed it, and whether it
/// carries an e-mail.
///
/// All three are decisions the bootstrap has to make before writing:
/// - the instance id, because the persons-seed matches accounts on the whole
///   triple (`source_type`, `source_id`, `account_id`) and a binding written
///   under any other one would never be recognised as this account's;
/// - closure, because an account gone from its source must not open a door;
/// - the e-mail, because an account that HAS one is the batch's to link — it
///   groups by e-mail, and minting a fresh person for such an account races
///   the link and splits one human across two persons.
///
/// `GROUP BY` is load-bearing, not tidiness: a bare aggregate returns ONE row
/// over an empty match set, carrying zero values, which would read as a real
/// answer and turn "nothing observed this account" into "observed".
const OBSERVED_ACCOUNT_SQL: &str = r"
    SELECT
        toString(argMax(insight_source_id, _synced_at))  AS source_id,
        argMax(ifNull(operation_type, ''), _synced_at)   AS latest_op,
        argMaxIf(
            ifNull(value, ''), _synced_at,
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

/// One account as the login bootstrap needs to see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedAccount {
    pub source_id: Uuid,
    /// The latest event is a closure signal — deactivated at the source.
    pub is_closed: bool,
    /// The address the connectors carry for it, if any. `Some` means the
    /// persons-seed can link this account by itself.
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
    })
}

/// A source that sends the parts but no whole name still names the person.
fn compose_name(first: Option<String>, last: Option<String>) -> Option<String> {
    match (first, last) {
        (Some(first), Some(last)) => Some(format!("{first} {last}")),
        (Some(one), None) | (None, Some(one)) => Some(one),
        (None, None) => None,
    }
}

impl ClickHouseEvidenceReader {
    /// Accounts whose current values contain `needle`, newest-agnostic.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails or a stored source id is not a UUID.
    pub async fn search(&self, needle: &str, limit: u64) -> anyhow::Result<Vec<AccountEvidence>> {
        let sql = SEARCH_SQL.replace("{FOLD}", FOLD_SQL);
        let rows: Vec<FoldedRow> = match self
            .client
            .query(&sql)
            .bind(needle)
            .bind(needle)
            .bind(needle)
            .bind(needle)
            .bind(limit)
            .fetch_all()
            .await
        {
            Ok(rows) => rows,
            Err(e) if is_missing_relation(&e) => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };

        // A row with an unreadable source id is one unusable account, not a
        // reason to answer nothing — the same rule the full fold applies.
        Ok(rows
            .into_iter()
            .filter_map(|row| map_row(row).ok())
            .collect())
    }

    /// Whether the evidence knows this account.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
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
            // A fresh install has no evidence relation yet — dbt creates it on
            // its first silver build. That is "nothing observed", not a failure
            // to answer.
            Err(e) if is_missing_relation(&e) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// The account as the connectors last described it, or `None` when none
    /// has seen it. See [`SOURCE_ID_SQL`] for why the caller needs the
    /// instance id rather than one of its own choosing.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails or the stored id is not a UUID.
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
            // Same reading as `has_account`: no relation yet is "nothing
            // observed", not a failure to answer.
            Err(e) if is_missing_relation(&e) => None,
            Err(e) => return Err(e.into()),
        };

        // An account nothing has observed yields no row at all. A blank or
        // all-zero stored id is not an instance id either: a binding written
        // under one would be invisible to the seed's account matching, so it
        // reads as "no such account" rather than as an answer.
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

/// ClickHouse reports an absent table as `UNKNOWN_TABLE` (code 60) — and a
/// fresh install lacks the whole `identity` database until its first build,
/// which reads as `UNKNOWN_DATABASE` (code 81). Both mean "nothing observed
/// yet", never a failure to answer.
fn is_missing_relation(error: &clickhouse::error::Error) -> bool {
    let message = error.to_string();
    message.contains("UNKNOWN_TABLE")
        || message.contains("code: 60")
        || message.contains("UNKNOWN_DATABASE")
        || message.contains("code: 81")
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
        }
    }

    #[test]
    fn an_absent_database_reads_as_no_observations_like_an_absent_table() {
        // A fresh install has neither `identity.identity_inputs` nor the
        // `identity` database itself — dbt creates both on its first build.
        for (label, message) in [
            (
                "absent table",
                "Code: 60. DB::Exception: ... (UNKNOWN_TABLE)",
            ),
            (
                "absent database",
                "Code: 81. DB::Exception: Database identity does not exist. (UNKNOWN_DATABASE)",
            ),
        ] {
            let error = clickhouse::error::Error::BadResponse(message.to_owned());
            assert!(
                is_missing_relation(&error),
                "should read as missing: {label}"
            );
        }

        let real = clickhouse::error::Error::BadResponse(
            "Code: 241. DB::Exception: Memory limit exceeded".to_owned(),
        );
        assert!(!is_missing_relation(&real), "a real failure must surface");
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
}
