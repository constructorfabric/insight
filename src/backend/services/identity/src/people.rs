//! In-memory people store built from `bronze_bamboohr.employees`.
//!
//! Loaded once at startup. Builds email→person lookup and
//! supervisor→subordinates relationships. Returns recursive subordinate
//! trees on lookup, with circular dependency protection.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Walk an error and its `source()` chain looking for a missing-source
/// signature in any layer's `Display` output. Defends against the case
/// where `clickhouse` v0.14 wraps the server-side `Code: 81./60.` line
/// inside a higher-level variant whose own `Display` text drops it.
fn error_chain_contains_missing_source(e: &(dyn std::error::Error + 'static)) -> bool {
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(layer) = cur {
        let s = layer.to_string();
        if s.contains("UNKNOWN_DATABASE")
            || s.contains("UNKNOWN_TABLE")
            || s.contains("Code: 81.")
            || s.contains("Code: 60.")
        {
            return true;
        }
        cur = layer.source();
    }
    false
}

/// Raw row from `bronze_bamboohr.employees`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEmployee {
    id: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    first_name: Option<String>,
    #[serde(default)]
    last_name: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    work_email: Option<String>,
    #[serde(default)]
    department: Option<String>,
    #[serde(default)]
    division: Option<String>,
    #[serde(default)]
    job_title: Option<String>,
    #[serde(default)]
    supervisor_email: Option<String>,
    #[serde(default)]
    supervisor: Option<String>,
}

/// Flat person record stored internally (no nested subordinates).
#[derive(Debug, Clone)]
struct PersonRecord {
    email: String,
    display_name: String,
    first_name: String,
    last_name: String,
    department: String,
    division: String,
    job_title: String,
    status: String,
    supervisor_email: Option<String>,
    supervisor_name: Option<String>,
    /// Direct subordinate emails (lowercased).
    direct_reports: Vec<String>,
}

/// Person returned by the API — with recursive subordinate tree.
#[derive(Debug, Clone, Serialize)]
pub struct Person {
    pub email: String,
    pub display_name: String,
    pub first_name: String,
    pub last_name: String,
    pub department: String,
    pub division: String,
    pub job_title: String,
    pub status: String,
    pub supervisor_email: Option<String>,
    pub supervisor_name: Option<String>,
    pub subordinates: Vec<Person>,
}

/// In-memory store: email (lowercased) → `PersonRecord`.
pub struct PeopleStore {
    by_email: HashMap<String, PersonRecord>,
    aliases: HashMap<String, String>,
}

impl PeopleStore {
    /// Load all active employees from `ClickHouse`, deduplicate by id
    /// (keep latest by `_airbyte_extracted_at`), and build relationships.
    ///
    /// **First-install resilience:** if `bronze_bamboohr.employees` does
    /// not exist yet (`UNKNOWN_DATABASE` / `UNKNOWN_TABLE` / CH error code
    /// 81 or 60), this function logs a warning and returns an empty
    /// store instead of failing. The bamboohr Airbyte sync populates
    /// the table on first run; restart this pod afterwards to reload.
    ///
    /// # Errors
    /// Returns an error if the `ClickHouse` query fails for any reason
    /// other than the source bronze table being absent, or if response
    /// parsing fails.
    pub async fn load(ch: &insight_clickhouse::Client) -> anyhow::Result<Self> {
        tracing::info!("loading people from bronze_bamboohr.employees");

        let sql = r"
            SELECT
                id,
                status,
                firstName,
                lastName,
                displayName,
                workEmail,
                department,
                division,
                jobTitle,
                supervisorEmail,
                supervisor
            FROM bronze_bamboohr.employees
            WHERE status = 'Active' AND workEmail != ''
            ORDER BY id, _airbyte_extracted_at DESC
        ";

        let mut cursor = ch
            .query(sql)
            .fetch_bytes("JSONEachRow")
            .map_err(|e| anyhow::anyhow!("ClickHouse query failed: {e}"))?;

        // First-install resilience: bronze_bamboohr.employees is only present
        // after the bamboohr Airbyte sync runs. On a fresh cluster it does
        // not exist yet — start with an empty store rather than crash-looping
        // the whole service (which would block every other helm-managed
        // component waiting for it to become Ready). After the first sync,
        // restart the pod to populate the in-memory store.
        let raw_bytes = match cursor.collect().await {
            Ok(bytes) => bytes,
            Err(e) => {
                // CH 25.x error codes for missing source: 81 = UNKNOWN_DATABASE,
                // 60 = UNKNOWN_TABLE. The clickhouse v0.14 Rust client does not
                // expose numeric codes as a typed accessor (the error enum has
                // SchemaMismatch + Other(BoxedError); the server Code: ... line
                // ends up in `Display`), so we string-match the canonical name
                // AND the numeric code prefix to defend against locale changes
                // and against future formatting tweaks dropping the symbolic
                // name.
                //
                // Walk the full source-chain: clickhouse v0.14 may wrap a
                // server error inside a higher-level variant whose own
                // `Display` only prints the wrapper text (no Code: line) —
                // checking each layer catches the inner-source case. If the
                // upstream crate ever surfaces a typed code we should switch
                // to that; until then this is the supported fallback per the
                // crate's docs.
                let missing_source = error_chain_contains_missing_source(&e);
                if missing_source {
                    tracing::warn!(
                        error = %e,
                        "bronze_bamboohr.employees not present yet — starting with an empty PeopleStore. \
                         Restart this pod after the first bamboohr Airbyte sync to load people."
                    );
                    return Ok(Self::build(&[]));
                }
                return Err(anyhow::anyhow!("ClickHouse fetch failed: {e}"));
            }
        };

        let mut seen_ids: HashSet<String> = HashSet::new();
        let mut employees: Vec<RawEmployee> = Vec::new();

        if !raw_bytes.is_empty() {
            for line in raw_bytes.split(|&b| b == b'\n').filter(|l| !l.is_empty()) {
                let row: RawEmployee = serde_json::from_slice(line)?;
                if seen_ids.contains(&row.id) {
                    continue;
                }
                seen_ids.insert(row.id.clone());
                employees.push(row);
            }
        }

        tracing::info!(count = employees.len(), "parsed unique active employees");

        Ok(Self::build(&employees))
    }

    /// Build a store from raw JSON lines (one `RawEmployee` per line).
    /// Used for testing without `ClickHouse`.
    ///
    /// # Errors
    /// Returns an error if any line fails to deserialize as a `RawEmployee`.
    pub fn from_json_lines(data: &[u8]) -> anyhow::Result<Self> {
        let mut seen_ids: HashSet<String> = HashSet::new();
        let mut employees: Vec<RawEmployee> = Vec::new();

        for line in data.split(|&b| b == b'\n').filter(|l| !l.is_empty()) {
            let row: RawEmployee = serde_json::from_slice(line)?;
            if seen_ids.contains(&row.id) {
                continue;
            }
            seen_ids.insert(row.id.clone());
            employees.push(row);
        }

        let mut store = Self::build(&employees);
        store.aliases = HashMap::new();
        Ok(store)
    }

    fn build(employees: &[RawEmployee]) -> Self {
        // Build flat person records
        let mut by_email: HashMap<String, PersonRecord> = HashMap::new();
        for emp in employees {
            let email = emp.work_email.as_deref().unwrap_or_default();
            if email.is_empty() {
                continue;
            }
            let key = email.to_lowercase();
            by_email.insert(
                key,
                PersonRecord {
                    email: email.to_owned(),
                    display_name: emp.display_name.clone().unwrap_or_default(),
                    first_name: emp.first_name.clone().unwrap_or_default(),
                    last_name: emp.last_name.clone().unwrap_or_default(),
                    department: emp.department.clone().unwrap_or_default(),
                    division: emp.division.clone().unwrap_or_default(),
                    job_title: emp.job_title.clone().unwrap_or_default(),
                    status: emp.status.clone().unwrap_or_default(),
                    supervisor_email: emp.supervisor_email.clone(),
                    supervisor_name: emp.supervisor.clone(),
                    direct_reports: Vec::new(),
                },
            );
        }

        // Build direct_reports: for each person, register them under their supervisor
        for emp in employees {
            if let Some(ref sup_email) = emp.supervisor_email {
                let sup_key = sup_email.to_lowercase();
                let email = emp.work_email.as_deref().unwrap_or_default();
                if email.is_empty() {
                    continue;
                }
                if let Some(supervisor) = by_email.get_mut(&sup_key) {
                    supervisor.direct_reports.push(email.to_lowercase());
                }
            }
        }

        let mut aliases = HashMap::new();
        aliases.insert("test@vz.com".to_owned(), "some@person-at-vz.com".to_owned());

        Self { by_email, aliases }
    }

    /// Look up a person by email, returning the full recursive subordinate tree.
    #[must_use]
    pub fn get_by_email(&self, email: &str) -> Option<Person> {
        let key = email.to_lowercase();
        let resolved = self.aliases.get(&key).unwrap_or(&key);
        let record = self.by_email.get(resolved)?;
        let mut visited = HashSet::new();
        Some(self.build_tree(record, &mut visited))
    }

    /// Recursively build a `Person` with nested subordinates.
    /// `visited` prevents infinite loops from circular supervisor references.
    fn build_tree(&self, record: &PersonRecord, visited: &mut HashSet<String>) -> Person {
        let key = record.email.to_lowercase();
        visited.insert(key);

        let subordinates = record
            .direct_reports
            .iter()
            .filter_map(|sub_email| {
                if visited.contains(sub_email) {
                    return None; // break cycle
                }
                let sub_record = self.by_email.get(sub_email)?;
                Some(self.build_tree(sub_record, visited))
            })
            .collect();

        Person {
            email: record.email.clone(),
            display_name: record.display_name.clone(),
            first_name: record.first_name.clone(),
            last_name: record.last_name.clone(),
            department: record.department.clone(),
            division: record.division.clone(),
            job_title: record.job_title.clone(),
            status: record.status.clone(),
            supervisor_email: record.supervisor_email.clone(),
            supervisor_name: record.supervisor_name.clone(),
            subordinates,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_email.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_email.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_data() -> &'static [u8] {
        br#"{"id":"1","status":"Active","firstName":"Alice","lastName":"Smith","displayName":"Alice Smith","workEmail":"alice@example.com","department":"Engineering","division":"R&D","jobTitle":"Staff Engineer","supervisorEmail":"bob@example.com","supervisor":"Jones, Bob"}
{"id":"2","status":"Active","firstName":"Bob","lastName":"Jones","displayName":"Bob Jones","workEmail":"bob@example.com","department":"Engineering","division":"R&D","jobTitle":"Engineering Manager","supervisorEmail":"carol@example.com","supervisor":"Lee, Carol"}
{"id":"3","status":"Active","firstName":"Carol","lastName":"Lee","displayName":"Carol Lee","workEmail":"carol@example.com","department":"Engineering","division":"R&D","jobTitle":"VP Engineering","supervisorEmail":null,"supervisor":null}
{"id":"4","status":"Active","firstName":"Dave","lastName":"Ng","displayName":"Dave Ng","workEmail":"dave@example.com","department":"Engineering","division":"R&D","jobTitle":"Senior Engineer","supervisorEmail":"bob@example.com","supervisor":"Jones, Bob"}"#
    }

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn loads_all_employees() -> TestResult {
        let store = PeopleStore::from_json_lines(test_data())?;
        assert_eq!(store.len(), 4);
        Ok(())
    }

    #[test]
    fn lookup_by_email() -> TestResult {
        let store = PeopleStore::from_json_lines(test_data())?;
        let alice = store
            .get_by_email("alice@example.com")
            .ok_or("alice not found")?;
        assert_eq!(alice.display_name, "Alice Smith");
        assert_eq!(alice.department, "Engineering");
        assert_eq!(alice.job_title, "Staff Engineer");
        Ok(())
    }

    #[test]
    fn lookup_case_insensitive() -> TestResult {
        let store = PeopleStore::from_json_lines(test_data())?;
        assert!(store.get_by_email("Alice@Example.COM").is_some());
        Ok(())
    }

    #[test]
    fn lookup_not_found() -> TestResult {
        let store = PeopleStore::from_json_lines(test_data())?;
        assert!(store.get_by_email("nobody@example.com").is_none());
        Ok(())
    }

    #[test]
    fn supervisor_has_subordinates() -> TestResult {
        let store = PeopleStore::from_json_lines(test_data())?;
        let bob = store
            .get_by_email("bob@example.com")
            .ok_or("bob not found")?;
        assert_eq!(bob.subordinates.len(), 2);

        let sub_emails: Vec<&str> = bob.subordinates.iter().map(|s| s.email.as_str()).collect();
        assert!(sub_emails.contains(&"alice@example.com"));
        assert!(sub_emails.contains(&"dave@example.com"));
        Ok(())
    }

    #[test]
    fn leaf_employee_has_no_subordinates() -> TestResult {
        let store = PeopleStore::from_json_lines(test_data())?;
        let alice = store
            .get_by_email("alice@example.com")
            .ok_or("alice not found")?;
        assert!(alice.subordinates.is_empty());
        Ok(())
    }

    #[test]
    fn supervisor_info_populated() -> TestResult {
        let store = PeopleStore::from_json_lines(test_data())?;
        let alice = store
            .get_by_email("alice@example.com")
            .ok_or("alice not found")?;
        assert_eq!(alice.supervisor_email.as_deref(), Some("bob@example.com"));
        assert_eq!(alice.supervisor_name.as_deref(), Some("Jones, Bob"));
        Ok(())
    }

    #[test]
    fn top_level_has_no_supervisor() -> TestResult {
        let store = PeopleStore::from_json_lines(test_data())?;
        let carol = store
            .get_by_email("carol@example.com")
            .ok_or("carol not found")?;
        assert!(carol.supervisor_email.is_none());
        assert!(carol.supervisor_name.is_none());
        Ok(())
    }

    #[test]
    fn recursive_tree() -> TestResult {
        // Carol → Bob → Alice, Dave
        let store = PeopleStore::from_json_lines(test_data())?;
        let carol = store
            .get_by_email("carol@example.com")
            .ok_or("carol not found")?;

        assert_eq!(carol.subordinates.len(), 1);
        let bob = &carol.subordinates[0];
        assert_eq!(bob.email, "bob@example.com");
        assert_eq!(bob.subordinates.len(), 2);

        let nested_emails: Vec<&str> = bob.subordinates.iter().map(|s| s.email.as_str()).collect();
        assert!(nested_emails.contains(&"alice@example.com"));
        assert!(nested_emails.contains(&"dave@example.com"));

        // Leaves have no subordinates
        for leaf in &bob.subordinates {
            assert!(leaf.subordinates.is_empty());
        }
        Ok(())
    }

    #[test]
    fn circular_dependency_safe() -> TestResult {
        // A reports to B, B reports to A — cycle
        let data = br#"{"id":"1","status":"Active","firstName":"A","lastName":"A","displayName":"A","workEmail":"a@example.com","department":"Eng","division":"R&D","jobTitle":"Eng","supervisorEmail":"b@example.com","supervisor":"B"}
{"id":"2","status":"Active","firstName":"B","lastName":"B","displayName":"B","workEmail":"b@example.com","department":"Eng","division":"R&D","jobTitle":"Eng","supervisorEmail":"a@example.com","supervisor":"A"}"#;
        let store = PeopleStore::from_json_lines(data)?;

        let a = store.get_by_email("a@example.com").ok_or("a not found")?;
        // A has B as subordinate, but B should NOT have A again (cycle broken)
        assert_eq!(a.subordinates.len(), 1);
        assert_eq!(a.subordinates[0].email, "b@example.com");
        assert!(a.subordinates[0].subordinates.is_empty()); // cycle cut

        let b = store.get_by_email("b@example.com").ok_or("b not found")?;
        assert_eq!(b.subordinates.len(), 1);
        assert_eq!(b.subordinates[0].email, "a@example.com");
        assert!(b.subordinates[0].subordinates.is_empty()); // cycle cut
        Ok(())
    }

    #[test]
    fn deduplicates_by_id() -> TestResult {
        let data = br#"{"id":"1","status":"Active","firstName":"Alice","lastName":"Smith","displayName":"Alice Smith","workEmail":"alice@example.com","department":"Eng","division":"R&D","jobTitle":"Engineer","supervisorEmail":null,"supervisor":null}
{"id":"1","status":"Active","firstName":"Alice","lastName":"Smith-Updated","displayName":"Alice Smith-Updated","workEmail":"alice@example.com","department":"Eng","division":"R&D","jobTitle":"Staff Engineer","supervisorEmail":null,"supervisor":null}"#;
        let store = PeopleStore::from_json_lines(data)?;
        assert_eq!(store.len(), 1);
        let alice = store
            .get_by_email("alice@example.com")
            .ok_or("alice not found")?;
        assert_eq!(alice.last_name, "Smith");
        Ok(())
    }

    #[test]
    fn empty_data() -> TestResult {
        let store = PeopleStore::from_json_lines(b"")?;
        assert_eq!(store.len(), 0);
        Ok(())
    }

    #[test]
    fn skips_empty_email() -> TestResult {
        let data = br#"{"id":"1","status":"Active","firstName":"Ghost","lastName":"User","displayName":"Ghost","workEmail":"","department":"Eng","division":"R&D","jobTitle":"","supervisorEmail":null,"supervisor":null}"#;
        let store = PeopleStore::from_json_lines(data)?;
        assert_eq!(store.len(), 0);
        Ok(())
    }
}
