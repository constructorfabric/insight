//! The statements this module sends, and the single place the relations they
//! read are named.
//!
//! The rules the inline relations reproduce belong to the identity mapping
//! (`src/ingestion/dbt/macros/resolve_person_id.sql`); they are written out
//! here only because the mapping does not yet publish them as relations of its
//! own. When it does, [`MAPPING`] becomes
//! [`MappingRelations::PublishedViews`] and nothing else in this crate moves.

/// The reserved person meaning "not a human" — bots, CI and service accounts
/// are bound to it, and it is unmintable because UUIDv7 never produces an
/// all-ones value.
const EXCLUDED_PERSON: &str = "ffffffff-ffff-ffff-ffff-ffffffffffff";

/// Where this module reads the identity mapping from.
///
/// Both variants answer the same two questions, project the same columns and
/// take the same single bound parameter, so the swap is the whole migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MappingRelations {
    /// The mapping's rules read straight off `identity.identity_inputs` and
    /// `identity.identity_persons`.
    InlineIdentityTables,
    /// The mapping's own published views.
    PublishedViews,
}

/// INVARIANT: whichever relations are in force, each projects `person_id` plus
/// one identity value column and carries exactly one `?` for the tenant. The
/// requested people narrow the answer in [`mapping_sql`], outside the relation.
pub(super) const MAPPING: MappingRelations = MappingRelations::InlineIdentityTables;

impl MappingRelations {
    /// `(email, person_id)` — one row per email that resolves to somebody.
    pub(super) fn email_claims(self) -> String {
        match self {
            Self::InlineIdentityTables => inline_email_claims(),
            Self::PublishedViews => published_view("email", "identity.person_map"),
        }
    }

    /// `(account_id, person_id)` — the current binding of every source account.
    pub(super) fn account_bindings(self) -> String {
        match self {
            Self::InlineIdentityTables => inline_account_bindings(),
            Self::PublishedViews => published_view("account_id", "identity.account_assignment"),
        }
    }
}

/// Per person, the identity values a dataset's entity column can hold for them.
///
/// INVARIANT: `WHERE person_id IN ?` sits outside the email claims, after their
/// dual-claimant `HAVING`. Narrowing the claims to the requested people first
/// would hide the second claimant of a shared email and award that email to the
/// first — the one outcome the mapping's rules exist to prevent.
///
/// INVARIANT: both value columns are coalesced. The columns underneath are
/// `Nullable(String)`, and a nullable element type makes the row decode refuse
/// the array — a refusal only a real server raises.
pub(super) fn mapping_sql() -> String {
    let mut sql = vec![
        "SELECT".to_owned(),
        "    person_id,".to_owned(),
        "    arraySort(groupUniqArrayIf(value, kind = 'email')) AS emails,".to_owned(),
        "    arraySort(groupUniqArrayIf(value, kind = 'account_id')) AS account_ids".to_owned(),
        "FROM (".to_owned(),
        "    SELECT".to_owned(),
        "        person_id,".to_owned(),
        "        'email' AS kind,".to_owned(),
        "        coalesce(email, '') AS value".to_owned(),
        "    FROM (".to_owned(),
    ];

    sql.extend(indent(&MAPPING.email_claims(), 8));
    sql.extend(
        [
            "    ) AS email_map",
            "    WHERE person_id IN ?",
            "    UNION ALL",
            "    SELECT",
            "        person_id,",
            "        'account_id' AS kind,",
            "        coalesce(account_id, '') AS value",
            "    FROM (",
        ]
        .map(ToOwned::to_owned),
    );

    sql.extend(indent(&MAPPING.account_bindings(), 8));
    sql.extend(
        [
            "    ) AS account_map",
            "    WHERE person_id IN ?",
            ") AS claimed",
            "GROUP BY person_id",
            "ORDER BY person_id",
        ]
        .map(ToOwned::to_owned),
    );

    sql.join("\n")
}

/// How fresh the mapping is, in milliseconds since the epoch.
///
/// Each mapping store carries one monotonic marker: the journal's mirror
/// timestamp, and the observation log's `ReplacingMergeTree` version. The
/// greater of the two advances whenever either store is written, which is what
/// a cursor needs — it may say "newer" more often than the map really changed,
/// but it never says "unchanged" about a map that did.
///
/// The observation log is deliberately unfiltered: it carries a producer-side
/// tenant that never equals the journal's, so no predicate over it means what
/// it appears to, and no join between the two stores may use one.
pub(super) const EPOCH_SQL: &str = "SELECT toUInt64(greatest(\n    \
     (SELECT max(toUnixTimestamp64Milli(_synced_at)) FROM identity.identity_persons \
     WHERE insight_tenant_id = toUUID(?)),\n    \
     (SELECT max(_version) FROM identity.identity_inputs)\n\
     )) AS epoch";

fn published_view(value_column: &str, view: &str) -> String {
    [
        "SELECT".to_owned(),
        format!("    {value_column},"),
        "    person_id".to_owned(),
        format!("FROM {view}"),
        "WHERE insight_tenant_id = toUUID(?)".to_owned(),
    ]
    .join("\n")
}

/// The mapping's account-derived email claim, read in the direction it is
/// written: every email an account has carried claims it, the account's latest
/// binding names the claimant, a claim by the excluded person is no claim at
/// all, and an email two people claim resolves to nobody.
fn inline_email_claims() -> String {
    let mut sql = [
        "SELECT",
        "    ae.email AS email,",
        "    any(cb.person_id) AS person_id",
        "FROM (",
        "    SELECT DISTINCT",
        "        insight_source_type AS source_type,",
        "        insight_source_id AS source_id,",
        "        source_account_id AS account_id,",
        "        lower(trimBoth(value)) AS email",
        "    FROM identity.identity_inputs",
        "    WHERE value_type = 'email'",
        "      AND operation_type = 'UPSERT'",
        "      AND coalesce(value, '') != ''",
        "      AND coalesce(source_account_id, '') != ''",
        ") AS ae",
        "INNER JOIN (",
    ]
    .map(ToOwned::to_owned)
    .to_vec();

    sql.extend(indent(&current_bindings("trimBoth(value_effective)"), 4));
    sql.extend(
        [
            ") AS cb",
            "    ON cb.source_type = ae.source_type",
            "   AND cb.source_id = ae.source_id",
            "   AND cb.account_id = ae.account_id",
            "WHERE ae.email != ''",
        ]
        .map(ToOwned::to_owned),
    );
    sql.push(format!("  AND cb.person_id != toUUID('{EXCLUDED_PERSON}')"));
    sql.extend(["GROUP BY ae.email", "HAVING uniqExact(cb.person_id) = 1"].map(ToOwned::to_owned));

    sql.join("\n")
}

/// The same bindings the email claim joins through, keyed for a fact that
/// carries the source's own account id rather than an address.
///
/// A binding to the excluded person stays: read forward it terminates
/// resolution, and read backwards it can only ever appear under the excluded
/// person itself, never under a human.
fn inline_account_bindings() -> String {
    current_bindings("lower(trimBoth(value_effective))")
}

/// The latest `value_type = 'id'` row per `(source_type, source_id,
/// account_id)` — the binding that decides who an account is.
///
/// The account key is normalized differently on the two sides the mapping
/// serves: the email claim joins the observation log's raw `source_account_id`,
/// while a fact's account column is matched case-insensitively.
fn current_bindings(account_id: &str) -> String {
    [
        "SELECT".to_owned(),
        "    insight_source_type AS source_type,".to_owned(),
        "    insight_source_id AS source_id,".to_owned(),
        format!("    {account_id} AS account_id,"),
        "    person_id".to_owned(),
        "FROM identity.identity_persons".to_owned(),
        "WHERE value_type = 'id'".to_owned(),
        "  AND insight_tenant_id = toUUID(?)".to_owned(),
        "  AND value_effective IS NOT NULL".to_owned(),
        "  AND trimBoth(value_effective) != ''".to_owned(),
        "ORDER BY source_type, source_id, account_id, created_at DESC, id DESC".to_owned(),
        "LIMIT 1 BY source_type, source_id, account_id".to_owned(),
    ]
    .join("\n")
}

fn indent(sql: &str, width: usize) -> Vec<String> {
    let pad = " ".repeat(width);
    sql.lines().map(|line| format!("{pad}{line}")).collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    /// What each `?` in a statement stands for, read off the text in front of
    /// it. The bind order in `mod.rs` has to match this sequence exactly.
    fn placeholders(sql: &str) -> Vec<&'static str> {
        sql.match_indices('?')
            .map(|(at, _)| {
                let before = &sql[..at];
                if before.ends_with("toUUID(") {
                    "tenant"
                } else if before.ends_with("person_id IN ") {
                    "people"
                } else {
                    "unrecognized"
                }
            })
            .collect()
    }

    fn quoted_literals(sql: &str) -> BTreeSet<&str> {
        sql.split('\'').skip(1).step_by(2).collect()
    }

    #[test]
    fn the_mapping_binds_the_tenant_and_the_person_set_once_per_relation() {
        assert_eq!(
            placeholders(&mapping_sql()),
            vec!["tenant", "people", "tenant", "people"]
        );
    }

    #[test]
    fn every_relation_the_mapping_can_read_takes_the_tenant_and_nothing_else() {
        for relations in [
            MappingRelations::InlineIdentityTables,
            MappingRelations::PublishedViews,
        ] {
            for (question, sql) in [
                ("email claims", relations.email_claims()),
                ("account bindings", relations.account_bindings()),
            ] {
                assert_eq!(
                    placeholders(&sql),
                    vec!["tenant"],
                    "should bind the tenant alone: {relations:?} {question}"
                );
                assert!(
                    sql.contains("person_id"),
                    "should project the person: {relations:?} {question}"
                );
            }
        }
    }

    #[test]
    fn the_requested_people_narrow_the_claims_only_after_the_shared_email_rule() {
        let sql = mapping_sql();

        assert!(
            sql.find("HAVING uniqExact(cb.person_id) = 1") < sql.find("WHERE person_id IN ?"),
            "narrowing first awards a shared email to one of its claimants: {sql}"
        );
    }

    #[test]
    fn an_excluded_binding_claims_no_email() {
        let sql = inline_email_claims();

        assert!(
            sql.find(EXCLUDED_PERSON) < sql.find("GROUP BY ae.email"),
            "an excluded claimant counted into the group unresolves the email: {sql}"
        );
    }

    #[test]
    fn the_statement_carries_no_value_of_its_own_beyond_the_reserved_person() {
        let sql = mapping_sql();

        assert_eq!(
            quoted_literals(&sql),
            BTreeSet::from(["", "UPSERT", "account_id", "email", "id", EXCLUDED_PERSON])
        );
    }

    #[test]
    fn both_identity_values_decode_as_plain_strings() {
        let sql = mapping_sql();

        assert!(sql.contains("coalesce(email, '') AS value"), "{sql}");
        assert!(sql.contains("coalesce(account_id, '') AS value"), "{sql}");
    }

    #[test]
    fn each_account_contributes_its_latest_binding_only() {
        let sql = inline_account_bindings();

        assert!(
            sql.contains("ORDER BY source_type, source_id, account_id, created_at DESC, id DESC"),
            "{sql}"
        );
        assert!(
            sql.contains("LIMIT 1 BY source_type, source_id, account_id"),
            "{sql}"
        );
    }

    #[test]
    fn the_account_key_is_normalized_per_side_the_mapping_serves() {
        assert!(
            inline_email_claims().contains("trimBoth(value_effective) AS account_id"),
            "the observation log's raw account id is not lowercased on either side"
        );
        assert!(
            inline_account_bindings().contains("lower(trimBoth(value_effective)) AS account_id"),
            "a fact's account column is matched case-insensitively"
        );
    }

    #[test]
    fn the_epoch_binds_the_tenant_and_reads_both_mapping_stores() {
        assert_eq!(placeholders(EPOCH_SQL), vec!["tenant"]);
        assert!(
            EPOCH_SQL.contains("identity.identity_persons"),
            "{EPOCH_SQL}"
        );
        assert!(
            EPOCH_SQL.contains("identity.identity_inputs"),
            "{EPOCH_SQL}"
        );
    }

    #[test]
    fn the_mapping_statement_reads_as_written() {
        assert_eq!(
            mapping_sql(),
            [
                "SELECT",
                "    person_id,",
                "    arraySort(groupUniqArrayIf(value, kind = 'email')) AS emails,",
                "    arraySort(groupUniqArrayIf(value, kind = 'account_id')) AS account_ids",
                "FROM (",
                "    SELECT",
                "        person_id,",
                "        'email' AS kind,",
                "        coalesce(email, '') AS value",
                "    FROM (",
                "        SELECT",
                "            ae.email AS email,",
                "            any(cb.person_id) AS person_id",
                "        FROM (",
                "            SELECT DISTINCT",
                "                insight_source_type AS source_type,",
                "                insight_source_id AS source_id,",
                "                source_account_id AS account_id,",
                "                lower(trimBoth(value)) AS email",
                "            FROM identity.identity_inputs",
                "            WHERE value_type = 'email'",
                "              AND operation_type = 'UPSERT'",
                "              AND coalesce(value, '') != ''",
                "              AND coalesce(source_account_id, '') != ''",
                "        ) AS ae",
                "        INNER JOIN (",
                "            SELECT",
                "                insight_source_type AS source_type,",
                "                insight_source_id AS source_id,",
                "                trimBoth(value_effective) AS account_id,",
                "                person_id",
                "            FROM identity.identity_persons",
                "            WHERE value_type = 'id'",
                "              AND insight_tenant_id = toUUID(?)",
                "              AND value_effective IS NOT NULL",
                "              AND trimBoth(value_effective) != ''",
                "            ORDER BY source_type, source_id, account_id, created_at DESC, id DESC",
                "            LIMIT 1 BY source_type, source_id, account_id",
                "        ) AS cb",
                "            ON cb.source_type = ae.source_type",
                "           AND cb.source_id = ae.source_id",
                "           AND cb.account_id = ae.account_id",
                "        WHERE ae.email != ''",
                "          AND cb.person_id != toUUID('ffffffff-ffff-ffff-ffff-ffffffffffff')",
                "        GROUP BY ae.email",
                "        HAVING uniqExact(cb.person_id) = 1",
                "    ) AS email_map",
                "    WHERE person_id IN ?",
                "    UNION ALL",
                "    SELECT",
                "        person_id,",
                "        'account_id' AS kind,",
                "        coalesce(account_id, '') AS value",
                "    FROM (",
                "        SELECT",
                "            insight_source_type AS source_type,",
                "            insight_source_id AS source_id,",
                "            lower(trimBoth(value_effective)) AS account_id,",
                "            person_id",
                "        FROM identity.identity_persons",
                "        WHERE value_type = 'id'",
                "          AND insight_tenant_id = toUUID(?)",
                "          AND value_effective IS NOT NULL",
                "          AND trimBoth(value_effective) != ''",
                "        ORDER BY source_type, source_id, account_id, created_at DESC, id DESC",
                "        LIMIT 1 BY source_type, source_id, account_id",
                "    ) AS account_map",
                "    WHERE person_id IN ?",
                ") AS claimed",
                "GROUP BY person_id",
                "ORDER BY person_id",
            ]
            .join("\n")
        );
    }
}
