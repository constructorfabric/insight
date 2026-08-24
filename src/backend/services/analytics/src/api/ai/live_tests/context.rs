//! `/v1/ai/context` — a person's own entries and the organisation's.

use super::*;
#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn a_person_writes_reads_and_removes_their_own_context() -> TestResult {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    let tenant = Uuid::now_v7();

    let created = app(db.clone(), tenant, enabled_config())
        .oneshot(json_req(
            "POST",
            "/v1/ai/context",
            &json!({ "scope": "person", "title": "How my week runs", "body": "Meeting-heavy midweek." }),
        )?)
        .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = body_json(created).await?;
    let id = created["id"].as_str().unwrap_or_default().to_owned();
    assert_eq!(created["scope"], "person");

    let listed = app(db.clone(), tenant, enabled_config())
        .oneshot(get("/v1/ai/context")?)
        .await?;
    let items = body_json(listed).await?;
    assert_eq!(items["items"].as_array().map(Vec::len), Some(1));

    let edited = app(db.clone(), tenant, enabled_config())
        .oneshot(json_req(
            "PATCH",
            &format!("/v1/ai/context/{id}"),
            &json!({ "title": "How my week actually runs" }),
        )?)
        .await?;
    assert_eq!(
        body_json(edited).await?["title"],
        "How my week actually runs"
    );

    let removed = app(db.clone(), tenant, enabled_config())
        .oneshot(empty_req("DELETE", &format!("/v1/ai/context/{id}"))?)
        .await?;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);

    let after = app(db, tenant, enabled_config())
        .oneshot(get("/v1/ai/context")?)
        .await?;
    assert_eq!(
        body_json(after).await?["items"].as_array().map(Vec::len),
        Some(0)
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn an_entry_with_no_title_is_refused() -> TestResult {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };

    let resp = app(db, Uuid::now_v7(), enabled_config())
        .oneshot(json_req(
            "POST",
            "/v1/ai/context",
            &json!({ "scope": "person", "title": "  ", "body": "something" }),
        )?)
        .await?;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn organisation_context_is_not_a_persons_to_write() -> TestResult {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };

    let resp = app(db, Uuid::now_v7(), enabled_config())
        .oneshot(json_req(
            "POST",
            "/v1/ai/context",
            &json!({ "scope": "tenant", "title": "Ours", "body": "How we read metrics." }),
        )?)
        .await?;

    assert_eq!(
        resp.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "an unreachable identity must fail closed on an admin-gated write"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn editing_someone_elses_entry_reads_as_absent() -> TestResult {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };

    let resp = app(db, Uuid::now_v7(), enabled_config())
        .oneshot(json_req(
            "PATCH",
            &format!("/v1/ai/context/{}", Uuid::now_v7()),
            &json!({ "title": "mine now" }),
        )?)
        .await?;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    Ok(())
}
