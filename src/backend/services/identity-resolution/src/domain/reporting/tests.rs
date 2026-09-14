use super::*;
use crate::domain::seed::IdentityInputRow;

#[test]
fn source_changes_and_clears_replace_redirects_but_repeated_or_missing_evidence_does_not()
-> anyhow::Result<()> {
    let account = SourceAccountKey {
        source_type: "directory".to_owned(),
        source_id: Uuid::from_u128(1),
        account_id: "report".to_owned(),
    };
    let source_parent = Uuid::from_u128(2);
    let corrected_parent = Uuid::from_u128(3);
    let new_parent = Uuid::from_u128(4);
    let previous = ReportingLine {
        child: Uuid::from_u128(5),
        source_type: account.source_type.clone(),
        source_id: account.source_id,
        parent: Some(corrected_parent),
        reference: Some(ManagerReference::RedirectedPerson {
            source_person_id: source_parent,
            person_id: corrected_parent,
        }),
    };
    for (value, deleted, expected) in [
        (
            Some(source_parent.to_string()),
            false,
            Some(corrected_parent),
        ),
        (None, false, Some(corrected_parent)),
        (Some(new_parent.to_string()), false, Some(new_parent)),
        (Some(String::new()), true, None),
        (Some(String::new()), false, None),
    ] {
        let observations = value
            .as_ref()
            .map(|value| IdentityInputRow {
                source_type: account.source_type.clone(),
                source_id: account.source_id,
                source_account_id: account.account_id.clone(),
                value_type: "parent_person_id".to_owned(),
                value: value.clone(),
                synced_at: chrono::DateTime::UNIX_EPOCH.naive_utc(),
                is_delete: deleted,
            })
            .into_iter()
            .collect();
        let profile = SeedProfile {
            account: account.clone(),
            latest_email: None,
            is_closed: false,
            roster_membership: None,
            observations,
        };
        let projected = project_profile(
            previous.child,
            &account,
            Some(&profile),
            Some(&previous),
            &HashMap::new(),
            &HashMap::new(),
        )?;
        assert_eq!(
            projected.parent, expected,
            "value={value:?}, deleted={deleted}"
        );
        let encoded = serde_json::to_string(&projected.reference)?;
        assert_eq!(
            serde_json::from_str::<Option<ManagerReference>>(&encoded)?,
            projected.reference
        );
    }
    Ok(())
}

#[test]
fn missing_evidence_does_not_clear_a_legacy_manager_without_a_reference() {
    let account = SourceAccountKey {
        source_type: "directory".to_owned(),
        source_id: Uuid::from_u128(1),
        account_id: "report".to_owned(),
    };
    let previous = ReportingLine {
        child: Uuid::from_u128(2),
        source_type: account.source_type.clone(),
        source_id: account.source_id,
        parent: Some(Uuid::from_u128(3)),
        reference: None,
    };
    assert!(matches!(
        project_profile(
            previous.child,
            &account,
            None,
            Some(&previous),
            &HashMap::new(),
            &HashMap::new()
        ),
        Err(ReportingError::UnresolvedReference)
    ));
}
