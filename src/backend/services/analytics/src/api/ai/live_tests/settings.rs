use super::*;
#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn a_tenant_without_its_own_prompt_reads_the_shipped_one() -> TestResult {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };

    let resp = app(db, Uuid::now_v7(), enabled_config())
        .oneshot(get("/v1/ai/settings")?)
        .await?;

    let body = body_json(resp).await?;
    assert_eq!(body["is_default"], true);
    assert!(
        body["system_prompt"]
            .as_str()
            .unwrap_or_default()
            .contains("explain"),
        "the shipped prompt should describe the job"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn writing_the_prompt_needs_an_admin_check_that_can_answer() -> TestResult {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };

    let resp = app(db, Uuid::now_v7(), enabled_config())
        .oneshot(json_req(
            "PUT",
            "/v1/ai/settings",
            &json!({ "system_prompt": "ours" }),
        )?)
        .await?;

    assert_eq!(
        resp.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "an unreachable identity must fail closed, never permit"
    );
    Ok(())
}
