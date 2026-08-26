use super::*;
#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn explaining_without_a_stored_key_says_so() -> TestResult {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };

    let resp = app(db, Uuid::now_v7(), enabled_config())
        .oneshot(json_req("POST", "/v1/ai/explain", &snapshot())?)
        .await?;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await?;
    assert!(
        serde_json::to_string(&body)?.contains("no Anthropic key"),
        "the refusal should name the missing key: {body}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn explaining_needs_an_admin_check_that_can_answer() -> TestResult {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };

    let resp = app(db, Uuid::now_v7(), admin_gated_config())
        .oneshot(json_req("POST", "/v1/ai/explain", &snapshot())?)
        .await?;

    assert_ne!(
        resp.status(),
        StatusCode::OK,
        "an unreachable identity must fail closed on an admin-gated explain"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn a_stand_key_answers_for_a_caller_who_stored_none() -> TestResult {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };

    // Nothing is stored for this tenant, so without the stand's own key this
    // is the 400 the test above asserts. Reaching the upstream instead is the
    // evidence that the stand key was used.
    let resp = app(db, Uuid::now_v7(), stand_key_config())
        .oneshot(json_req("POST", "/v1/ai/explain", &snapshot())?)
        .await?;

    assert_ne!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "a stand key means no caller needs one of their own"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live MariaDB (INTEGRATION_TESTS_MARIADB_URL)"]
async fn an_unreachable_model_reads_as_busy_rather_than_broken() -> TestResult {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    let tenant = Uuid::now_v7();

    let stored = app(db.clone(), tenant, enabled_config())
        .oneshot(json_req(
            "PUT",
            "/v1/ai/credentials",
            &json!({ "token": TOKEN }),
        )?)
        .await?;
    assert_eq!(stored.status(), StatusCode::OK);

    let resp = app(db, tenant, enabled_config())
        .oneshot(json_req("POST", "/v1/ai/explain", &snapshot())?)
        .await?;

    assert!(
        resp.status().is_client_error() || resp.status().is_server_error(),
        "an unreachable upstream cannot produce an answer"
    );
    assert_ne!(
        resp.status(),
        StatusCode::OK,
        "no answer may be invented when the model was never reached"
    );
    Ok(())
}

fn snapshot() -> Value {
    json!({
        "metric_key": "tasks.closed",
        "label": "Tasks closed",
        "value": "34",
        "period": "month",
        "since": "2026-08-01",
        "until": "2026-08-22",
        "delta": "+6 since last month",
        "peer": "Team median 27",
        "help": "",
        "trend": [1.0, null, 3.0],
    })
}
