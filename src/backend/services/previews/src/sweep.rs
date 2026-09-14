//! TTL sweep: a plain interval task (the service runs a single replica) that
//! deletes every experiment past its `expires-at` annotation. Spawned once at
//! gear init; a failed pass logs and waits for the next tick.

use std::time::Duration;

use chrono::Utc;

use crate::domain::experiment::ExperimentStatus;
use crate::domain::objects::experiment_from_deployment;
use crate::infra::cluster::Cluster;

pub async fn run(cluster: Cluster, interval_secs: u64) {
    let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs.max(1)));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        ticker.tick().await;
        if let Err(e) = sweep_once(&cluster).await {
            tracing::warn!(error = %format!("{e:#}"), "TTL sweep pass failed");
        }
    }
}

async fn sweep_once(cluster: &Cluster) -> anyhow::Result<()> {
    let now = Utc::now();
    let deployments = cluster.list_experiment_deployments().await?;

    for deployment in &deployments {
        let Some(experiment) = experiment_from_deployment(deployment, now) else {
            continue;
        };
        if experiment.status != ExperimentStatus::Expired {
            continue;
        }

        let Some(resource_name) = deployment.metadata.name.clone() else {
            continue;
        };
        match cluster.delete_trio(&resource_name).await {
            Ok(_) => {
                tracing::info!(
                    name = experiment.name,
                    expired_at = ?experiment.expires_at,
                    "TTL sweep removed an expired experiment"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %format!("{e:#}"),
                    name = experiment.name,
                    "TTL sweep failed to remove an expired experiment"
                );
            }
        }
    }
    Ok(())
}
