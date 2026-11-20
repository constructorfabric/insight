use super::seed_repo;
use super::test_fixture::fixture_or_skip;

type TestResult = anyhow::Result<()>;

#[tokio::test]
async fn an_address_is_owned_by_the_latest_person_to_state_it() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let shared = format!("shared.{}@emailmap.test", f.tenant.simple());
    let leaver = f
        .person(&format!("leaver.{}@emailmap.test", f.tenant.simple()))
        .await?;
    let joiner = f
        .person(&format!("joiner.{}@emailmap.test", f.tenant.simple()))
        .await?;
    f.observed_at(joiner, "email", &shared, 60).await?;
    f.observed_at(leaver, "email", &shared, 86_400).await?;

    let map = seed_repo::latest_email_to_person(&f.db, f.tenant).await?;

    assert_eq!(
        map.get(&shared),
        Some(&joiner),
        "the address must be owned by the person who stated it last"
    );
    Ok(())
}

#[tokio::test]
async fn an_address_is_keyed_in_lower_case() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let stated = format!("Ivan.Petrov.{}@EmailMap.test", f.tenant.simple());
    let person = f.person(&stated).await?;

    let map = seed_repo::latest_email_to_person(&f.db, f.tenant).await?;

    assert_eq!(
        map.get(&stated.to_lowercase()),
        Some(&person),
        "callers look the address up lowercased, so that is the key that must exist"
    );
    assert!(
        !map.contains_key(&stated),
        "keeping the case the source stated would give the same address two keys"
    );
    Ok(())
}

#[tokio::test]
async fn another_tenants_address_is_absent_from_this_tenants_map() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let other = f.in_another_tenant();
    let addr = format!("elsewhere.{}@emailmap.test", other.tenant.simple());
    let ours = format!("ours.{}@emailmap.test", f.tenant.simple());
    other.person(&addr).await?;
    let mine = f.person(&ours).await?;

    let map = seed_repo::latest_email_to_person(&f.db, f.tenant).await?;

    assert!(
        !map.contains_key(&addr),
        "the map stops at the tenant that asked for it"
    );
    assert_eq!(
        map.get(&ours),
        Some(&mine),
        "while this tenant's own address is in it"
    );
    Ok(())
}

#[tokio::test]
async fn an_address_the_source_left_blank_is_not_a_key() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let addr = format!("blank.{}@emailmap.test", f.tenant.simple());
    let person = f.person(&addr).await?;
    f.observed(person, "email", "").await?;

    let map = seed_repo::latest_email_to_person(&f.db, f.tenant).await?;

    assert!(
        !map.contains_key(""),
        "a blank address is an absent value, not an identity every blank row shares"
    );
    assert_eq!(
        map.get(&addr),
        Some(&person),
        "the blank row must not displace the address the person does hold"
    );
    Ok(())
}
