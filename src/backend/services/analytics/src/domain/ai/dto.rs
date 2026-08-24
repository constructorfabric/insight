//! Wire shapes for the AI-assist surface, and the parsing that keeps a raw
//! string from reaching a table or a prompt.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::migration::ai_assist_schema;

/// Who a context entry belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Written by an admin, read into every explanation in the tenant.
    Tenant,
    /// Written by one person, read into their own explanations only.
    Person,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tenant => "tenant",
            Self::Person => "person",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "tenant" => Some(Self::Tenant),
            "person" => Some(Self::Person),
            _ => None,
        }
    }
}

/// Why a piece of authored text cannot be stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRejected {
    Empty,
    TooLong { max: usize },
}

impl TextRejected {
    pub fn reason(self) -> String {
        match self {
            Self::Empty => "must not be empty".to_owned(),
            Self::TooLong { max } => format!("must be at most {max} characters"),
        }
    }
}

/// A context entry's title, within the column budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Title(String);

impl Title {
    pub fn parse(raw: &str) -> Result<Self, TextRejected> {
        let max = ai_assist_schema::TITLE as usize;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(TextRejected::Empty);
        }
        if trimmed.chars().count() > max {
            return Err(TextRejected::TooLong { max });
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

/// Authored prose — a context entry's body, or a tenant's system prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prose(String);

impl Prose {
    pub fn parse(raw: &str) -> Result<Self, TextRejected> {
        let max = ai_assist_schema::BODY;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(TextRejected::Empty);
        }
        if trimmed.chars().count() > max {
            return Err(TextRejected::TooLong { max });
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

/// Longest Anthropic key we will accept — generous, and still a bound.
const MAX_TOKEN_CHARS: usize = 256;

/// Why a submitted key cannot be stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenRejected {
    Empty,
    TooLong,
    NotPrintableAscii,
}

impl TokenRejected {
    pub fn reason(self) -> String {
        match self {
            Self::Empty => "must not be empty".to_owned(),
            Self::TooLong => format!("must be at most {MAX_TOKEN_CHARS} characters"),
            Self::NotPrintableAscii => "must be printable ASCII with no spaces".to_owned(),
        }
    }
}

/// An Anthropic key on its way to being sealed. Never serialized.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiToken(String);

impl ApiToken {
    pub fn parse(raw: &str) -> Result<Self, TokenRejected> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(TokenRejected::Empty);
        }
        if trimmed.len() > MAX_TOKEN_CHARS {
            return Err(TokenRejected::TooLong);
        }
        if !trimmed
            .bytes()
            .all(|b| b.is_ascii_graphic() && !b.is_ascii_whitespace())
        {
            return Err(TokenRejected::NotPrintableAscii);
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// SAFETY: the whole point of the newtype is that a key never reaches a log line.
impl std::fmt::Debug for ApiToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiToken(redacted)")
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AiConfigResponse {
    /// Whether this deployment offers AI explanations at all.
    pub enabled: bool,
    /// The model explanations are asked of.
    pub model: String,
    /// The stand pays for explanations with its own key, so nobody stores one.
    pub stand_key: bool,
    /// Only admins may ask for an explanation on this deployment.
    pub admin_only: bool,
}
impl toolkit::api::api_dto::ResponseApiDto for AiConfigResponse {}

#[derive(Debug, Serialize, ToSchema)]
pub struct AiCredentialResponse {
    pub configured: bool,
    /// Last four characters of the stored key; empty when none is stored.
    pub hint: String,
}
impl toolkit::api::api_dto::ResponseApiDto for AiCredentialResponse {}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PutCredentialRequest {
    pub token: String,
}
impl toolkit::api::api_dto::RequestApiDto for PutCredentialRequest {}

#[derive(Debug, Serialize, ToSchema)]
pub struct AiSettingsResponse {
    pub system_prompt: String,
    /// True while the tenant has written none of its own.
    pub is_default: bool,
}
impl toolkit::api::api_dto::ResponseApiDto for AiSettingsResponse {}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PutSettingsRequest {
    pub system_prompt: String,
}
impl toolkit::api::api_dto::RequestApiDto for PutSettingsRequest {}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContextEntryResponse {
    pub id: String,
    pub scope: Scope,
    pub title: String,
    pub body: String,
    pub updated_at: String,
}
impl toolkit::api::api_dto::ResponseApiDto for ContextEntryResponse {}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContextListResponse {
    pub items: Vec<ContextEntryResponse>,
}
impl toolkit::api::api_dto::ResponseApiDto for ContextListResponse {}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateContextRequest {
    pub scope: Scope,
    pub title: String,
    pub body: String,
}
impl toolkit::api::api_dto::RequestApiDto for CreateContextRequest {}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateContextRequest {
    pub title: Option<String>,
    pub body: Option<String>,
}
impl toolkit::api::api_dto::RequestApiDto for UpdateContextRequest {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_rejects_blank_input() {
        assert_eq!(Title::parse("   "), Err(TextRejected::Empty));
    }

    #[test]
    fn title_rejects_input_over_the_column_budget() {
        let long = "x".repeat(ai_assist_schema::TITLE as usize + 1);

        assert!(matches!(
            Title::parse(&long),
            Err(TextRejected::TooLong { .. })
        ));
    }

    #[test]
    fn prose_rejects_input_over_the_product_budget() {
        let long = "x".repeat(ai_assist_schema::BODY + 1);

        assert!(matches!(
            Prose::parse(&long),
            Err(TextRejected::TooLong { .. })
        ));
    }

    #[test]
    fn prose_keeps_the_trimmed_text() -> Result<(), TextRejected> {
        assert_eq!(Prose::parse("  hello  ")?.into_inner(), "hello");
        Ok(())
    }

    #[test]
    fn token_rejects_a_value_with_spaces() {
        assert_eq!(
            ApiToken::parse("sk-ant nope"),
            Err(TokenRejected::NotPrintableAscii)
        );
    }

    #[test]
    fn token_debug_never_prints_the_value() -> Result<(), TokenRejected> {
        let token = ApiToken::parse("sk-ant-secret-value")?;

        assert_eq!(format!("{token:?}"), "ApiToken(redacted)");
        Ok(())
    }

    #[test]
    fn scope_round_trips_through_its_wire_value() {
        assert_eq!(Scope::parse(Scope::Tenant.as_str()), Some(Scope::Tenant));
        assert_eq!(Scope::parse(Scope::Person.as_str()), Some(Scope::Person));
        assert_eq!(Scope::parse("everyone"), None);
    }
}
