use super::*;
use crate::domain::provenance::Provenance;

#[test]
fn unrelated_roster_accounts_do_not_consume_the_correction_evidence_budget() -> anyhow::Result<()> {
    let bindings: HashMap<_, _> = (1..=10_001_u128)
        .map(|id| {
            (
                SourceAccountKey {
                    source_type: "directory".to_owned(),
                    source_id: Uuid::from_u128(20_000),
                    account_id: id.to_string(),
                },
                KnownBinding {
                    person_id: Uuid::from_u128(id),
                    author_person_id: Uuid::nil(),
                    provenance: Provenance::Resolved,
                },
            )
        })
        .collect();
    let roster =
        RosterSource::parse("directory").ok_or_else(|| anyhow::anyhow!("missing roster"))?;
    let accounts = roster_accounts(
        &bindings,
        &HashSet::from([Uuid::from_u128(1), Uuid::from_u128(2)]),
        Some(&roster),
    );
    let selected: HashSet<_> = accounts
        .iter()
        .map(|account| account.account_id.as_str())
        .collect();
    assert_eq!(selected, HashSet::from(["1", "2"]));
    Ok(())
}
