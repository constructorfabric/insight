//! The statements this module sends, and the single place the relations they
//! read are named. The rules the inline relations reproduce belong to the
//! identity mapping (`src/ingestion/dbt/macros/resolve_person_id.sql`).

/// The reserved person meaning "not a human"; unmintable because UUIDv7 never
/// produces an all-ones value.
const EXCLUDED_PERSON: &str = "ffffffff-ffff-ffff-ffff-ffffffffffff";

/// Where this module reads the identity mapping from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MappingRelations {
    /// The mapping's rules, read straight off the `identity` tables.
    InlineIdentityTables,
    /// The mapping's own published views.
    PublishedViews,
}

/// INVARIANT: whichever relations are in force, each projects `person_id` plus
/// one identity value column; the journal tables carry one `?` for the tenant
/// each, the published views carry none.
pub(super) const MAPPING: MappingRelations = MappingRelations::PublishedViews;

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

/// INVARIANT: `WHERE person_id IN ?` sits outside the email claims, after their
/// dual-claimant `HAVING` — narrowing first would award a shared email to its
/// first claimant.
///
/// INVARIANT: both value columns are coalesced; the columns underneath are
/// `Nullable(String)`, and a nullable element type makes the row decode refuse
/// the array.
pub(super) fn mapping_sql(scope: MappingScope) -> String {
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
    sql.push("    ) AS email_map".to_owned());
    sql.extend(scope.narrowing().map(ToOwned::to_owned));
    sql.extend(
        [
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
    sql.push("    ) AS account_map".to_owned());
    sql.extend(scope.narrowing().map(ToOwned::to_owned));
    sql.extend([") AS claimed", "GROUP BY person_id", "ORDER BY person_id"].map(ToOwned::to_owned));
    sql.extend(scope.ceiling().map(ToOwned::to_owned));

    sql.join("\n")
}

/// Which people one mapping read answers for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MappingScope {
    /// The people the caller named; their count is the caller's own bound.
    RequestedPeople,
    /// Everyone the mapping resolves, up to a bound row ceiling.
    EveryPerson,
}

impl MappingScope {
    fn narrowing(self) -> Option<&'static str> {
        match self {
            Self::RequestedPeople => Some("    WHERE person_id IN ?"),
            Self::EveryPerson => None,
        }
    }

    fn ceiling(self) -> Option<&'static str> {
        match self {
            Self::RequestedPeople => None,
            Self::EveryPerson => Some("LIMIT ?"),
        }
    }
}

/// How fresh the mapping is, in milliseconds since the epoch. The greater of
/// the two stores' markers may say "newer" more often than the map moved, but
/// never says "unchanged" about a map that did.
///
/// SAFETY: the observation log is unfiltered on purpose — it carries a
/// producer-side tenant that never equals the journal's.
///
/// INVARIANT: a store with no marker of its own contributes 0, so the epoch
/// stays a plain `UInt64` the row decode accepts.
pub(super) const EPOCH_SQL: &str = "SELECT toUInt64(greatest(\n    \
     coalesce((SELECT max(toUnixTimestamp64Milli(_synced_at)) FROM identity.identity_persons \
     WHERE insight_tenant_id = toUUID(?)), 0),\n    \
     coalesce((SELECT max(_version) FROM identity.identity_inputs), 0)\n\
     )) AS epoch";

fn published_view(value_column: &str, view: &str) -> String {
    [
        "SELECT".to_owned(),
        format!("    {value_column},"),
        "    person_id".to_owned(),
        format!("FROM {view}"),
    ]
    .join("\n")
}

/// Every email an account has carried claims it, the account's latest binding
/// names the claimant, and an email two people claim resolves to nobody.
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
/// SAFETY: no exclusion filter here — read backwards, a binding to the excluded
/// person can only appear under that person, never under a human.
fn inline_account_bindings() -> String {
    current_bindings("lower(trimBoth(value_effective))")
}

/// The latest `value_type = 'id'` row per `(source_type, source_id,
/// account_id)` — the binding that decides who an account is.
///
/// INVARIANT: the account key is normalized per caller — the email claim joins
/// the raw `source_account_id`, a fact's account column matches case-insensitively.
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

    /// What each `?` stands for; the bind order in `mod.rs` must match it.
    fn placeholders(sql: &str) -> Vec<&'static str> {
        sql.match_indices('?')
            .map(|(at, _)| {
                let before = &sql[..at];
                if before.ends_with("toUUID(") {
                    "tenant"
                } else if before.ends_with("person_id IN ") {
                    "people"
                } else if before.ends_with("LIMIT ") {
                    "row ceiling"
                } else {
                    "unrecognized"
                }
            })
            .collect()
    }

    fn quoted_literals(sql: &str) -> BTreeSet<&str> {
        sql.split('\'').skip(1).step_by(2).collect()
    }

    /// What one mapping relation contributes to that bind order under the
    /// relations in force.
    fn relation_binds() -> Vec<&'static str> {
        match MAPPING {
            MappingRelations::InlineIdentityTables => vec!["tenant"],
            MappingRelations::PublishedViews => Vec::new(),
        }
    }

    #[test]
    fn each_relation_binds_its_own_placeholders_then_the_person_set() {
        let expected = [relation_binds(), vec!["people"]].concat().repeat(2);

        assert_eq!(
            placeholders(&mapping_sql(MappingScope::RequestedPeople)),
            expected
        );
    }

    #[test]
    fn enumerating_every_person_drops_the_person_narrowing_and_binds_a_row_ceiling() {
        let expected = [relation_binds().repeat(2), vec!["row ceiling"]].concat();

        assert_eq!(
            placeholders(&mapping_sql(MappingScope::EveryPerson)),
            expected
        );
    }

    #[test]
    fn both_scopes_read_the_same_claims_under_the_same_rules() {
        let named = mapping_sql(MappingScope::RequestedPeople);
        let every = mapping_sql(MappingScope::EveryPerson);

        let dropped: BTreeSet<&str> = named
            .lines()
            .filter(|line| !every.contains(*line))
            .collect();
        let added: BTreeSet<&str> = every
            .lines()
            .filter(|line| !named.contains(*line))
            .collect();

        assert_eq!(dropped, BTreeSet::from(["    WHERE person_id IN ?"]));
        assert_eq!(added, BTreeSet::from(["LIMIT ?"]));
        assert!(every.contains(&indent(&MAPPING.email_claims(), 8).join("\n")));
    }

    #[test]
    fn the_journal_relations_take_the_tenant_alone_and_the_published_views_take_nothing() {
        for (relations, expected) in [
            (MappingRelations::InlineIdentityTables, &["tenant"][..]),
            (MappingRelations::PublishedViews, &[][..]),
        ] {
            for (question, sql) in [
                ("email claims", relations.email_claims()),
                ("account bindings", relations.account_bindings()),
            ] {
                assert_eq!(
                    placeholders(&sql),
                    expected,
                    "should bind its own placeholders only: {relations:?} {question}"
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
        let sql = mapping_sql(MappingScope::RequestedPeople);

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
    fn the_statement_carries_no_value_of_its_own_beyond_the_claim_kinds() {
        let sql = mapping_sql(MappingScope::RequestedPeople);

        let email_claims = MAPPING.email_claims();
        let account_bindings = MAPPING.account_bindings();

        let mut expected = BTreeSet::from(["", "account_id", "email"]);
        expected.extend(quoted_literals(&email_claims));
        expected.extend(quoted_literals(&account_bindings));

        assert_eq!(quoted_literals(&sql), expected);
    }

    #[test]
    fn both_identity_values_decode_as_plain_strings() {
        let sql = mapping_sql(MappingScope::RequestedPeople);

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

    /// The complete epoch statement, as it reads.
    fn epoch_statement() -> String {
        [
            "SELECT toUInt64(greatest(",
            "    coalesce((SELECT max(toUnixTimestamp64Milli(_synced_at)) FROM identity.identity_persons WHERE insight_tenant_id = toUUID(?)), 0),",
            "    coalesce((SELECT max(_version) FROM identity.identity_inputs), 0)",
            ")) AS epoch",
        ]
        .join("\n")
    }

    #[test]
    fn the_epoch_statement_reads_as_written() {
        assert_eq!(EPOCH_SQL, epoch_statement());
    }

    #[test]
    fn a_store_with_no_marker_reads_zero_so_the_epoch_stays_a_plain_integer() {
        for store in [
            "(SELECT max(toUnixTimestamp64Milli(_synced_at)) FROM identity.identity_persons WHERE insight_tenant_id = toUUID(?)), 0)",
            "(SELECT max(_version) FROM identity.identity_inputs), 0)",
        ] {
            assert!(
                EPOCH_SQL.contains(&format!("coalesce({store}")),
                "an unread store leaves the epoch nullable, which no row decodes: {store}"
            );
        }
    }

    /// The complete statement the published views render, as it reads.
    fn published_mapping_statement() -> String {
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
            "            email,",
            "            person_id",
            "        FROM identity.person_map",
            "    ) AS email_map",
            "    WHERE person_id IN ?",
            "    UNION ALL",
            "    SELECT",
            "        person_id,",
            "        'account_id' AS kind,",
            "        coalesce(account_id, '') AS value",
            "    FROM (",
            "        SELECT",
            "            account_id,",
            "            person_id",
            "        FROM identity.account_assignment",
            "    ) AS account_map",
            "    WHERE person_id IN ?",
            ") AS claimed",
            "GROUP BY person_id",
            "ORDER BY person_id",
        ]
        .join("\n")
    }

    /// The complete statement the inline identity tables render, as it reads.
    fn inline_mapping_statement() -> String {
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
    }

    #[test]
    fn the_mapping_statement_reads_as_written() {
        let expected = match MAPPING {
            MappingRelations::InlineIdentityTables => inline_mapping_statement(),
            MappingRelations::PublishedViews => published_mapping_statement(),
        };

        assert_eq!(mapping_sql(MappingScope::RequestedPeople), expected);
    }

    #[test]
    fn each_pinned_statement_carries_the_relations_its_own_variant_renders() {
        for (relations, statement) in [
            (
                MappingRelations::InlineIdentityTables,
                inline_mapping_statement(),
            ),
            (
                MappingRelations::PublishedViews,
                published_mapping_statement(),
            ),
        ] {
            for (question, sql) in [
                ("email claims", relations.email_claims()),
                ("account bindings", relations.account_bindings()),
            ] {
                assert!(
                    statement.contains(&indent(&sql, 8).join("\n")),
                    "should read as written: {relations:?} {question}"
                );
            }
        }
    }
}
