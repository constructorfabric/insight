//! Kubernetes I/O for the experiment trio: create, delete, list — always in
//! the one configured namespace. Every call is timeout-bounded, and 404/409
//! are outcomes, not errors (same conventions as the gears-rust k8s
//! precedent).

use std::time::Duration;

use anyhow::{Context, anyhow};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Service;
use kube::api::{Api, DeleteParams, ListParams, PostParams};
use kube::{Client, Resource};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::domain::httproute::HttpRoute;
use crate::domain::objects::EXPERIMENT_LABEL;

const K8S_API_TIMEOUT: Duration = Duration::from_secs(10);

/// The one namespace this service operates in, with a client scoped to it.
#[derive(Clone)]
pub struct Cluster {
    client: Client,
    namespace: String,
}

/// Why a create did not happen.
#[derive(Debug)]
pub enum CreateError {
    /// An object of that name already exists (409 on the first object).
    AlreadyExists,
    Failed(anyhow::Error),
}

/// What a delete found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteOutcome {
    Deleted,
    /// None of the trio existed.
    NotFound,
}

impl Cluster {
    /// In-cluster (or kubeconfig) client. Fails boot when no Kubernetes
    /// environment is reachable — this service is nothing without one.
    pub async fn connect(namespace: String) -> anyhow::Result<Self> {
        let client = Client::try_default().await.context("kube client init")?;
        Ok(Self { client, namespace })
    }

    fn api<K>(&self) -> Api<K>
    where
        K: Resource<Scope = k8s_openapi::NamespaceResourceScope, DynamicType = ()>,
    {
        Api::namespaced(self.client.clone(), &self.namespace)
    }

    /// Every experiment Deployment in the namespace (label-selected); the
    /// Deployment carries the annotations and readiness the API serves.
    pub async fn list_experiment_deployments(&self) -> anyhow::Result<Vec<Deployment>> {
        let params = ListParams::default().labels(EXPERIMENT_LABEL);
        let list = tokio::time::timeout(K8S_API_TIMEOUT, self.api::<Deployment>().list(&params))
            .await
            .map_err(|_| anyhow!("timeout listing experiment deployments"))?
            .context("list experiment deployments")?;
        Ok(list.items)
    }

    /// Create the trio in order. A 409 on the Deployment means the experiment
    /// exists; a failure later rolls the already-created objects back
    /// (best-effort) so a half-experiment never lingers.
    pub async fn create_trio(
        &self,
        deployment: Deployment,
        service: Service,
        route: HttpRoute,
    ) -> Result<(), CreateError> {
        match self.create_one(&deployment).await {
            Created::Done => {}
            Created::AlreadyExists => return Err(CreateError::AlreadyExists),
            Created::Failed(e) => return Err(CreateError::Failed(e)),
        }

        // A 409 on a follow-up object is leftover state from a broken past
        // delete, not a second live experiment (the Deployment create above
        // was the existence check) — replacing it would need a read-modify
        // cycle, so surface it and roll back instead.
        for step in [
            self.create_one(&service).await,
            self.create_one(&route).await,
        ] {
            match step {
                Created::Done => {}
                Created::AlreadyExists => {
                    let cause = anyhow!("a leftover object of that name already exists");
                    return Err(self.rollback(&deployment, cause).await);
                }
                Created::Failed(e) => return Err(self.rollback(&deployment, e).await),
            }
        }
        Ok(())
    }

    async fn rollback(&self, deployment: &Deployment, cause: anyhow::Error) -> CreateError {
        let name = deployment.metadata.name.clone().unwrap_or_default();
        if let Err(e) = self.delete_trio(&name).await {
            tracing::error!(error = %format!("{e:#}"), %name, "rollback of a failed create left objects behind");
        }
        CreateError::Failed(cause.context("experiment create failed; rolled back"))
    }

    async fn create_one<K>(&self, object: &K) -> Created
    where
        K: Resource<Scope = k8s_openapi::NamespaceResourceScope, DynamicType = ()>
            + Serialize
            + DeserializeOwned
            + Clone
            + std::fmt::Debug,
    {
        let kind = K::kind(&()).into_owned();
        let api = self.api::<K>();
        match tokio::time::timeout(K8S_API_TIMEOUT, api.create(&PostParams::default(), object))
            .await
        {
            Ok(Ok(_)) => Created::Done,
            Ok(Err(kube::Error::Api(resp))) if resp.code == 409 => Created::AlreadyExists,
            Ok(Err(e)) => Created::Failed(anyhow!(e).context(format!("create {kind}"))),
            Err(_) => Created::Failed(anyhow!("timeout creating {kind}")),
        }
    }

    /// Delete the trio by resource name (`preview-<experiment>`); each 404 is
    /// tolerated, and all three missing reports [`DeleteOutcome::NotFound`].
    pub async fn delete_trio(&self, resource_name: &str) -> anyhow::Result<DeleteOutcome> {
        let deployment = self.delete_one::<Deployment>(resource_name).await?;
        let service = self.delete_one::<Service>(resource_name).await?;
        let route = self.delete_one::<HttpRoute>(resource_name).await?;

        if [deployment, service, route].contains(&DeleteOutcome::Deleted) {
            Ok(DeleteOutcome::Deleted)
        } else {
            Ok(DeleteOutcome::NotFound)
        }
    }

    async fn delete_one<K>(&self, name: &str) -> anyhow::Result<DeleteOutcome>
    where
        K: Resource<Scope = k8s_openapi::NamespaceResourceScope, DynamicType = ()>
            + DeserializeOwned
            + Clone
            + std::fmt::Debug,
    {
        let kind = K::kind(&()).into_owned();
        let api = self.api::<K>();
        match tokio::time::timeout(K8S_API_TIMEOUT, api.delete(name, &DeleteParams::default()))
            .await
        {
            Ok(Ok(_)) => Ok(DeleteOutcome::Deleted),
            Ok(Err(kube::Error::Api(resp))) if resp.code == 404 => Ok(DeleteOutcome::NotFound),
            Ok(Err(e)) => Err(anyhow!(e).context(format!("delete {kind} {name}"))),
            Err(_) => Err(anyhow!("timeout deleting {kind} {name}")),
        }
    }
}

enum Created {
    Done,
    AlreadyExists,
    Failed(anyhow::Error),
}
