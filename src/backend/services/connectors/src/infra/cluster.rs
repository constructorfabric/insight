use std::time::Duration;

use kube::api::{ApiResource, DynamicObject, GroupVersionKind, ListParams, Patch, PatchParams};
use kube::{Api, Client};
use serde_json::Value;

const SAVE_CALL_TIMEOUT: Duration = Duration::from_secs(3);
const CALL_TIMEOUT: Duration = Duration::from_secs(10);
const PAGE_SIZE: u32 = 100;
const MAX_OBJECTS: usize = 10_000;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("cluster request failed: {0}")]
    Api(#[from] kube::Error),
    #[error("cluster request timed out")]
    Timeout(#[from] tokio::time::error::Elapsed),
    #[error("cluster connection is unavailable")]
    Disabled,
    #[error("too many CronWorkflows")]
    Limit,
}

impl Error {
    pub(crate) fn refused(&self) -> bool {
        matches!(self, Self::Api(kube::Error::Api(response)) if (400..500).contains(&response.code))
            || matches!(self, Self::Disabled)
    }

    pub(crate) fn conflict(&self) -> bool {
        matches!(self, Self::Api(kube::Error::Api(response)) if response.code == 409)
    }

    pub(crate) fn server_error(&self) -> bool {
        matches!(self, Self::Api(kube::Error::Api(response)) if (500..600).contains(&response.code))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Cluster {
    api: Option<Api<DynamicObject>>,
    call_timeout: Duration,
    pub(crate) namespace: String,
    pub(crate) tenant_id: String,
    pub(crate) reconcile_workflow_name: String,
}

impl Cluster {
    pub(crate) fn disabled() -> Self {
        Self {
            api: None,
            call_timeout: CALL_TIMEOUT,
            namespace: String::new(),
            tenant_id: String::new(),
            reconcile_workflow_name: String::new(),
        }
    }

    pub(crate) fn new(client: Client, namespace: &str, tenant_id: &str) -> Self {
        let resource = ApiResource::from_gvk(&GroupVersionKind::gvk(
            "argoproj.io",
            "v1alpha1",
            "CronWorkflow",
        ));
        Self {
            api: Some(Api::namespaced_with(client, namespace, &resource)),
            namespace: namespace.to_owned(),
            tenant_id: tenant_id.to_owned(),
            ..Self::disabled()
        }
    }

    pub(crate) fn with_reconcile_workflow(mut self, name: String) -> Self {
        self.reconcile_workflow_name = name;
        self
    }

    pub(crate) fn for_save(&self) -> Self {
        Self {
            call_timeout: SAVE_CALL_TIMEOUT,
            ..self.clone()
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.api.is_some()
    }

    pub(crate) async fn get(&self, name: &str) -> Result<Option<DynamicObject>, Error> {
        let Some(api) = &self.api else {
            return Err(Error::Disabled);
        };
        Ok(tokio::time::timeout(self.call_timeout, api.get_opt(name)).await??)
    }

    pub(crate) async fn list(&self) -> Result<Vec<DynamicObject>, Error> {
        let Some(api) = &self.api else {
            return Err(Error::Disabled);
        };
        let mut params = ListParams::default()
            .labels("app.kubernetes.io/component=connector-sync")
            .limit(PAGE_SIZE);
        let mut objects = Vec::new();
        loop {
            let page = tokio::time::timeout(self.call_timeout, api.list(&params)).await??;
            objects.extend(page.items);
            if objects.len() > MAX_OBJECTS {
                return Err(Error::Limit);
            }
            let Some(token) = page.metadata.continue_.filter(|s| !s.is_empty()) else {
                return Ok(objects);
            };
            params = params.continue_token(&token);
        }
    }

    pub(crate) async fn patch(&self, name: &str, patch: &Value) -> Result<(), Error> {
        if let Some(api) = &self.api {
            let params = PatchParams {
                field_manager: Some("insight-connectors".to_owned()),
                ..PatchParams::default()
            };
            tokio::time::timeout(
                self.call_timeout,
                api.patch(name, &params, &Patch::Merge(patch)),
            )
            .await??;
        }
        Ok(())
    }
}
