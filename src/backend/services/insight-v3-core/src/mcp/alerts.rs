//! The alert tools: the same operations the API has, for a client that
//! speaks MCP.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use super::tools::{CustomSurfaces, parse_name, refuse};
use crate::api::alerts::{notification_response, rule_response};
use crate::domain::alerts::RuleDraft;
use crate::domain::alerts::rules::{AlertRules, AlertsError};
use crate::domain::definition::Page;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ListAlertsRequest {
    /// Text to look for in an alert's name or the metric it watches. Blank
    /// returns every alert.
    #[serde(default)]
    pub(crate) query: String,
    /// How many names to answer with: 1 to 200, 50 by default.
    pub(crate) limit: Option<u64>,
    /// How many names to skip, for the page after the first.
    pub(crate) offset: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct AlertNameRequest {
    /// Letters, digits, underscore and dash, up to 128 characters.
    pub(crate) name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct PutAlertRequest {
    /// The name to store under.
    pub(crate) name: String,
    /// The rule. Send `expected_revision` to replace an alert that exists.
    pub(crate) rule: RuleDraft,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SetAlertEnabledRequest {
    pub(crate) name: String,
    /// Whether checks should run.
    pub(crate) enabled: bool,
    /// The revision this expects to change, from `get_alert`.
    pub(crate) expected_revision: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct AlertNotificationsRequest {
    pub(crate) name: String,
    /// How many to answer with: 1 to 200, 50 by default.
    pub(crate) limit: Option<u64>,
    /// How many to skip, newest first.
    pub(crate) offset: Option<u64>,
}

impl CustomSurfaces {
    /// The alert operations, or the refusal that this installation has none.
    fn alerts(&self) -> Result<AlertRules<'_>, CallToolResult> {
        self.state()
            .alerts()
            .ok_or_else(|| refuse("alerts are not enabled on this installation"))
    }

    pub(crate) async fn alerts_list(&self, request: ListAlertsRequest) -> CallToolResult {
        let alerts = match self.alerts() {
            Ok(alerts) => alerts,
            Err(refusal) => return refusal,
        };
        let page = match Page::parse(request.limit, request.offset) {
            Ok(page) => page,
            Err(error) => return refuse(&error.to_string()),
        };

        match alerts.page(&request.query, page).await {
            Ok(found) => CallToolResult::structured(json!({
                "names": found.names,
                "total": found.total,
                "limit": page.limit(),
                "offset": page.offset(),
            })),
            Err(error) => alerts_error(&error),
        }
    }

    pub(crate) async fn alerts_get(&self, request: AlertNameRequest) -> CallToolResult {
        let alerts = match self.alerts() {
            Ok(alerts) => alerts,
            Err(refusal) => return refusal,
        };
        let name = match parse_name(&request.name) {
            Ok(name) => name,
            Err(refusal) => return refusal,
        };

        match alerts.get(&name).await {
            Ok(rule) => structured(&rule_response(&rule)),
            Err(error) => alerts_error(&error),
        }
    }

    pub(crate) async fn alerts_put(&self, request: PutAlertRequest) -> CallToolResult {
        let alerts = match self.alerts() {
            Ok(alerts) => alerts,
            Err(refusal) => return refusal,
        };
        let name = match parse_name(&request.name) {
            Ok(name) => name,
            Err(refusal) => return refusal,
        };
        match alerts.put(&name, &request.rule, None).await {
            Ok(rule) => structured(&rule_response(&rule)),
            Err(error) => alerts_error(&error),
        }
    }

    pub(crate) async fn alerts_set_enabled(
        &self,
        request: SetAlertEnabledRequest,
    ) -> CallToolResult {
        let alerts = match self.alerts() {
            Ok(alerts) => alerts,
            Err(refusal) => return refusal,
        };
        let name = match parse_name(&request.name) {
            Ok(name) => name,
            Err(refusal) => return refusal,
        };

        match alerts
            .set_enabled(&name, request.expected_revision, request.enabled)
            .await
        {
            Ok(rule) => structured(&rule_response(&rule)),
            Err(error) => alerts_error(&error),
        }
    }

    pub(crate) async fn alerts_delete(&self, request: AlertNameRequest) -> CallToolResult {
        let alerts = match self.alerts() {
            Ok(alerts) => alerts,
            Err(refusal) => return refusal,
        };
        let name = match parse_name(&request.name) {
            Ok(name) => name,
            Err(refusal) => return refusal,
        };

        match alerts.delete(&name).await {
            Ok(()) => CallToolResult::structured(json!({"deleted": request.name})),
            Err(error) => alerts_error(&error),
        }
    }

    pub(crate) async fn alerts_notifications(
        &self,
        request: AlertNotificationsRequest,
    ) -> CallToolResult {
        let alerts = match self.alerts() {
            Ok(alerts) => alerts,
            Err(refusal) => return refusal,
        };
        let name = match parse_name(&request.name) {
            Ok(name) => name,
            Err(refusal) => return refusal,
        };
        let page = match Page::parse(request.limit, request.offset) {
            Ok(page) => page,
            Err(error) => return refuse(&error.to_string()),
        };

        match alerts.notifications(&name, page).await {
            Ok(listed) => {
                let notifications: Vec<serde_json::Value> = listed
                    .iter()
                    .map(|notification| {
                        serde_json::to_value(notification_response(notification))
                            .unwrap_or(serde_json::Value::Null)
                    })
                    .collect();

                CallToolResult::structured(json!({
                    "notifications": notifications,
                    "limit": page.limit(),
                    "offset": page.offset(),
                }))
            }
            Err(error) => alerts_error(&error),
        }
    }

    pub(crate) fn alerts_destinations(&self) -> CallToolResult {
        let alerts = match self.alerts() {
            Ok(alerts) => alerts,
            Err(refusal) => return refusal,
        };
        let destinations: Vec<serde_json::Value> = alerts
            .destinations()
            .listed()
            .into_iter()
            .map(|(name, provider)| json!({"name": name, "provider": provider}))
            .collect();

        CallToolResult::structured(json!({ "destinations": destinations }))
    }
}

fn structured<T: serde::Serialize>(value: &T) -> CallToolResult {
    match serde_json::to_value(value) {
        Ok(value) => CallToolResult::structured(value),
        Err(error) => {
            tracing::error!(%error, "an alert could not be encoded");
            refuse("the alert could not be encoded")
        }
    }
}

fn alerts_error(error: &AlertsError) -> CallToolResult {
    if error.is_about_the_caller() {
        return refuse(&error.to_string());
    }

    tracing::error!(%error, "an alert tool call failed");

    refuse("the request could not be completed")
}
