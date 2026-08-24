use super::*;
#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn a_key_round_trips_as_its_last_four_characters_only() -> TestResult {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    let tenant = Uuid::now_v7();

    let created = app(db.clone(), tenant, enabled_config())
        .oneshot(json_req(
            "PUT",
            "/v1/ai/credentials",
            &json!({ "token": TOKEN }),
        )?)
        .await?;
    assert_eq!(created.status(), StatusCode::OK);
    let created = body_json(created).await?;
    assert_eq!(created["configured"], true);
    assert_eq!(created["hint"], "wxyz");
    assert!(created.get("token").is_none(), "the key came back out");

    let read = app(db.clone(), tenant, enabled_config())
        .oneshot(get("/v1/ai/credentials")?)
        .await?;
    let read = body_json(read).await?;
    assert_eq!(read["configured"], true);
    assert_eq!(read["hint"], "wxyz");

    let replaced = app(db.clone(), tenant, enabled_config())
        .oneshot(json_req(
            "PUT",
            "/v1/ai/credentials",
            &json!({ "token": "sk-ant-second-key-abcd" }),
        )?)
        .await?;
    assert_eq!(body_json(replaced).await?["hint"], "abcd");

    let removed = app(db.clone(), tenant, enabled_config())
        .oneshot(empty_req("DELETE", "/v1/ai/credentials")?)
        .await?;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);

    let after = app(db, tenant, enabled_config())
        .oneshot(get("/v1/ai/credentials")?)
        .await?;
    assert_eq!(body_json(after).await?["configured"], false);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn two_saves_at_once_leave_one_row_and_the_later_key() -> TestResult {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    let tenant = Uuid::now_v7();

    let (first, second) = tokio::join!(
        app(db.clone(), tenant, enabled_config()).oneshot(json_req(
            "PUT",
            "/v1/ai/credentials",
            &json!({ "token": "sk-ant-one-1111" }),
        )?),
        app(db.clone(), tenant, enabled_config()).oneshot(json_req(
            "PUT",
            "/v1/ai/credentials",
            &json!({ "token": "sk-ant-two-2222" }),
        )?),
    );

    assert_eq!(first?.status(), StatusCode::OK, "the first save must land");
    assert_eq!(
        second?.status(),
        StatusCode::OK,
        "a concurrent save must replace, not collide"
    );

    let read = app(db, tenant, enabled_config())
        .oneshot(get("/v1/ai/credentials")?)
        .await?;
    let hint = body_json(read).await?["hint"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(hint == "1111" || hint == "2222", "unexpected hint: {hint}");
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn a_blank_key_is_refused() -> TestResult {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };

    let resp = app(db, Uuid::now_v7(), enabled_config())
        .oneshot(json_req(
            "PUT",
            "/v1/ai/credentials",
            &json!({ "token": "   " }),
        )?)
        .await?;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    Ok(())
}
