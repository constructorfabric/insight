//! Wire-shape and gating rules that hold without a database.

use toolkit_canonical_errors::Problem;

use super::*;
use crate::domain::ai::dto::{AiCredentialResponse, Scope};

#[test]
fn a_disabled_stand_answers_not_found() -> Result<(), serde_json::Error> {
    let error = AiError::not_found("AI assistance is not enabled on this instance")
        .with_resource("ai_assist")
        .create();
    let problem = serde_json::to_value(Problem::from(error))?;

    assert_eq!(problem["status"], 404);
    Ok(())
}

#[test]
fn a_refused_context_write_names_the_role_it_wants() -> Result<(), serde_json::Error> {
    let problem = serde_json::to_value(Problem::from(admin_only_context()))?;

    assert_eq!(problem["status"], 403);
    assert_eq!(problem["context"]["reason"], ADMIN_ONLY_CONTEXT);
    Ok(())
}

#[test]
fn a_refused_prompt_write_names_the_role_it_wants() -> Result<(), serde_json::Error> {
    let problem = serde_json::to_value(Problem::from(admin_only_prompt()))?;

    assert_eq!(problem["status"], 403);
    assert_eq!(problem["context"]["reason"], ADMIN_ONLY_PROMPT);
    Ok(())
}

#[test]
fn the_credential_response_carries_no_token_field() -> Result<(), serde_json::Error> {
    let body = serde_json::to_value(AiCredentialResponse {
        configured: true,
        hint: "wxyz".to_owned(),
    })?;

    assert_eq!(
        body,
        serde_json::json!({ "configured": true, "hint": "wxyz" })
    );
    Ok(())
}

#[test]
fn the_context_scope_serializes_to_its_wire_value() -> Result<(), serde_json::Error> {
    assert_eq!(serde_json::to_value(Scope::Tenant)?, "tenant");
    assert_eq!(serde_json::to_value(Scope::Person)?, "person");
    Ok(())
}
