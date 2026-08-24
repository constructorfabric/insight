use super::*;
#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn config_reports_the_stand_switch_when_it_is_off() -> TestResult {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    let app = app(db, Uuid::now_v7(), GearConfig::default());

    let resp = app.oneshot(get("/v1/ai/config")?).await?;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await?["enabled"], false);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn a_stand_with_the_switch_off_hides_every_other_route() -> TestResult {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    let tenant = Uuid::now_v7();

    for uri in ["/v1/ai/credentials", "/v1/ai/settings", "/v1/ai/context"] {
        let app = app(db.clone(), tenant, GearConfig::default());
        let resp = app.oneshot(get(uri)?).await?;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{uri} must be hidden");
    }
    Ok(())
}
