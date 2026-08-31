//! Typed builders for the per-experiment object trio. Pure functions over
//! values, mirroring `deploy/preview/templates/*` — the manual chart stays the
//! shape reference, and `render_contract` tests in this crate guard the drift.

use std::collections::BTreeMap;

use chrono::{DateTime, SecondsFormat, Utc};
use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec};
use k8s_openapi::api::core::v1::{
    Container, ContainerPort, HTTPGetAction, PodSpec, PodTemplateSpec, Probe, ResourceRequirements,
    Service, ServicePort, ServiceSpec,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::ObjectMeta;

use super::experiment::{Experiment, ExperimentName, ImageTag, status_of};
use super::httproute::{
    self, BackendRef, HttpRoute, HttpRouteRule, HttpRouteSpec, ParentRef, PathMatch, RewritePath,
    RouteFilter, RouteMatch, UrlRewrite,
};

/// The one image previews may run. Hardcoded on purpose: the API accepts a tag
/// only, so there is no arbitrary-image surface.
pub const IMAGE_REPOSITORY: &str = "ghcr.io/constructorfabric/insight-frontend";

/// Label selecting every object of every experiment; the value is the slug.
pub const EXPERIMENT_LABEL: &str = "insight.dev/preview-experiment";

pub const CREATOR_ANNOTATION: &str = "insight.dev/preview-creator";
pub const CREATED_AT_ANNOTATION: &str = "insight.dev/preview-created-at";
pub const EXPIRES_AT_ANNOTATION: &str = "insight.dev/preview-expires-at";
pub const TAG_ANNOTATION: &str = "insight.dev/preview-image-tag";

/// The FE image serves on the non-root port; probes and the Service target it
/// by name (chart parity, PR #2400).
const CONTAINER_PORT: i32 = 8080;
const SERVICE_PORT: i32 = 80;

/// The shared Gateway an experiment's route attaches to, plus the host and
/// path prefix it serves under. From config; defaults mirror the chart.
#[derive(Debug, Clone)]
pub struct RouteTarget {
    pub gateway_name: String,
    pub gateway_namespace: String,
    /// Target listener on the Gateway (e.g. `https`). Empty = all listeners.
    pub gateway_section_name: String,
    /// The single preview host. One host serves every experiment.
    pub host: String,
    /// Prefix experiments live under; `/exp/<name>` is prefix-stripped to `/`.
    pub base_path: String,
}

/// Metadata stamped as annotations — Kubernetes is the only store, so the
/// creator, tag and lifetime live on the objects themselves.
#[derive(Debug, Clone)]
pub struct ExperimentStamp {
    pub creator: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

fn labels(name: &ExperimentName) -> BTreeMap<String, String> {
    let mut labels = selector_labels(name);
    labels.insert(
        "app.kubernetes.io/managed-by".to_owned(),
        "previews".to_owned(),
    );
    labels.insert(EXPERIMENT_LABEL.to_owned(), name.as_str().to_owned());
    labels
}

fn selector_labels(name: &ExperimentName) -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "app.kubernetes.io/name".to_owned(),
            "insight-preview".to_owned(),
        ),
        (
            "app.kubernetes.io/instance".to_owned(),
            name.resource_name(),
        ),
    ])
}

fn annotations(tag: &ImageTag, stamp: &ExperimentStamp) -> BTreeMap<String, String> {
    let rfc3339 = |t: &DateTime<Utc>| t.to_rfc3339_opts(SecondsFormat::Secs, true);
    BTreeMap::from([
        (CREATOR_ANNOTATION.to_owned(), stamp.creator.clone()),
        (CREATED_AT_ANNOTATION.to_owned(), rfc3339(&stamp.created_at)),
        (EXPIRES_AT_ANNOTATION.to_owned(), rfc3339(&stamp.expires_at)),
        (TAG_ANNOTATION.to_owned(), tag.as_str().to_owned()),
    ])
}

fn object_meta(name: &ExperimentName, tag: &ImageTag, stamp: &ExperimentStamp) -> ObjectMeta {
    ObjectMeta {
        name: Some(name.resource_name()),
        labels: Some(labels(name)),
        annotations: Some(annotations(tag, stamp)),
        ..ObjectMeta::default()
    }
}

/// The path prefix this experiment is served under, e.g. `/exp/<name>`.
fn route_path(base_path: &str, name: &ExperimentName) -> String {
    format!("{}/{}", base_path.trim_end_matches('/'), name.as_str())
}

fn healthz_probe(initial_delay: i32, period: i32) -> Probe {
    Probe {
        http_get: Some(HTTPGetAction {
            path: Some("/healthz".to_owned()),
            port: IntOrString::String("http".to_owned()),
            ..HTTPGetAction::default()
        }),
        initial_delay_seconds: Some(initial_delay),
        period_seconds: Some(period),
        ..Probe::default()
    }
}

fn fixed_resources() -> ResourceRequirements {
    let quantities = |cpu: &str, memory: &str| {
        BTreeMap::from([
            ("cpu".to_owned(), Quantity(cpu.to_owned())),
            ("memory".to_owned(), Quantity(memory.to_owned())),
        ])
    };
    ResourceRequirements {
        requests: Some(quantities("50m", "64Mi")),
        limits: Some(quantities("200m", "128Mi")),
        ..ResourceRequirements::default()
    }
}

/// The fixed pod shape: 1 replica, the hardcoded FE image at the validated
/// tag, `/healthz` probes on port 8080, pinned resources — and deliberately no
/// `env`/`command` (login is the gateway+authenticator's job; no injection
/// surface).
#[must_use]
pub fn deployment(name: &ExperimentName, tag: &ImageTag, stamp: &ExperimentStamp) -> Deployment {
    let container = Container {
        name: "frontend".to_owned(),
        image: Some(format!("{IMAGE_REPOSITORY}:{}", tag.as_str())),
        image_pull_policy: Some("IfNotPresent".to_owned()),
        ports: Some(vec![ContainerPort {
            name: Some("http".to_owned()),
            container_port: CONTAINER_PORT,
            protocol: Some("TCP".to_owned()),
            ..ContainerPort::default()
        }]),
        liveness_probe: Some(healthz_probe(5, 10)),
        readiness_probe: Some(healthz_probe(3, 5)),
        resources: Some(fixed_resources()),
        ..Container::default()
    };

    Deployment {
        metadata: object_meta(name, tag, stamp),
        spec: Some(DeploymentSpec {
            replicas: Some(1),
            selector: LabelSelector {
                match_labels: Some(selector_labels(name)),
                ..LabelSelector::default()
            },
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(selector_labels(name)),
                    ..ObjectMeta::default()
                }),
                spec: Some(PodSpec {
                    // Suppress the legacy `<SVC>_SERVICE_HOST/PORT` link
                    // env-vars kubelet injects for every Service in the
                    // namespace (chart parity).
                    enable_service_links: Some(false),
                    containers: vec![container],
                    ..PodSpec::default()
                }),
            },
            ..DeploymentSpec::default()
        }),
        ..Deployment::default()
    }
}

#[must_use]
pub fn service(name: &ExperimentName, tag: &ImageTag, stamp: &ExperimentStamp) -> Service {
    Service {
        metadata: object_meta(name, tag, stamp),
        spec: Some(ServiceSpec {
            type_: Some("ClusterIP".to_owned()),
            ports: Some(vec![ServicePort {
                name: Some("http".to_owned()),
                port: SERVICE_PORT,
                target_port: Some(IntOrString::String("http".to_owned())),
                protocol: Some("TCP".to_owned()),
                ..ServicePort::default()
            }]),
            selector: Some(selector_labels(name)),
            ..ServiceSpec::default()
        }),
        ..Service::default()
    }
}

/// Read an experiment record back from its Deployment — the annotations are
/// the metadata store, readiness is the status. `None` for an object without
/// the experiment label (not ours).
#[must_use]
pub fn experiment_from_deployment(
    deployment: &Deployment,
    now: DateTime<Utc>,
) -> Option<Experiment> {
    let meta = &deployment.metadata;
    let name = meta.labels.as_ref()?.get(EXPERIMENT_LABEL)?.clone();

    let annotation = |key: &str| {
        meta.annotations
            .as_ref()
            .and_then(|a| a.get(key))
            .cloned()
            .unwrap_or_default()
    };
    let timestamp = |key: &str| {
        DateTime::parse_from_rfc3339(&annotation(key))
            .ok()
            .map(|t| t.with_timezone(&Utc))
    };

    let expires_at = timestamp(EXPIRES_AT_ANNOTATION);
    let ready = deployment
        .status
        .as_ref()
        .and_then(|s| s.ready_replicas)
        .unwrap_or(0)
        >= 1;

    Some(Experiment {
        name,
        tag: annotation(TAG_ANNOTATION),
        creator: annotation(CREATOR_ANNOTATION),
        created_at: timestamp(CREATED_AT_ANNOTATION),
        expires_at,
        status: status_of(expires_at, ready, now),
    })
}

/// How many experiments count against the live cap: expired ones are already
/// condemned (the TTL sweep removes them on its next pass), so they must not
/// block a new create in the meantime.
#[must_use]
pub fn live_experiment_count(deployments: &[Deployment], now: DateTime<Utc>) -> usize {
    deployments
        .iter()
        .filter_map(|d| experiment_from_deployment(d, now))
        .filter(|e| e.status != super::experiment::ExperimentStatus::Expired)
        .count()
}

/// One route per experiment, attaching itself to the shared Gateway via
/// `parentRefs`: creating it ADDS the `/exp/<name>` path and deleting it
/// REMOVES it — no central config is rewritten. `URLRewrite
/// ReplacePrefixMatch /` strips the prefix before the FE pod.
#[must_use]
pub fn http_route(
    name: &ExperimentName,
    tag: &ImageTag,
    stamp: &ExperimentStamp,
    route: &RouteTarget,
) -> HttpRoute {
    let section_name = match route.gateway_section_name.as_str() {
        "" => None,
        section => Some(section.to_owned()),
    };

    HttpRoute {
        api_version: httproute::API_VERSION.to_owned(),
        kind: httproute::KIND.to_owned(),
        metadata: object_meta(name, tag, stamp),
        spec: HttpRouteSpec {
            parent_refs: vec![ParentRef {
                name: route.gateway_name.clone(),
                namespace: route.gateway_namespace.clone(),
                section_name,
            }],
            hostnames: vec![route.host.clone()],
            rules: vec![HttpRouteRule {
                matches: vec![RouteMatch {
                    path: PathMatch {
                        r#type: "PathPrefix".to_owned(),
                        value: route_path(&route.base_path, name),
                    },
                }],
                filters: vec![RouteFilter {
                    r#type: "URLRewrite".to_owned(),
                    url_rewrite: UrlRewrite {
                        path: RewritePath {
                            r#type: "ReplacePrefixMatch".to_owned(),
                            replace_prefix_match: "/".to_owned(),
                        },
                    },
                }],
                backend_refs: vec![BackendRef {
                    name: name.resource_name(),
                    port: SERVICE_PORT,
                }],
            }],
        },
    }
}
