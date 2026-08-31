//! Render contract: the object builders must produce the same three-object
//! shape as `deploy/preview/templates/*` — the same facts
//! `deploy/preview/tests/test_render.py` pins on the chart. Assertions run on
//! the serialized JSON so they check what the API server would receive.

use chrono::{TimeZone, Utc};
use serde_json::{Value, json};

use super::experiment::{ExperimentName, ImageTag};
use super::objects::{self, ExperimentStamp, RouteTarget};

type R = Result<(), Box<dyn std::error::Error>>;

fn name() -> ExperimentName {
    ExperimentName::parse("widget-alpha").unwrap_or_else(|e| panic!("{e}"))
}

fn tag() -> ImageTag {
    ImageTag::parse("preview-abc123").unwrap_or_else(|e| panic!("{e}"))
}

fn stamp() -> ExperimentStamp {
    let created_at = Utc
        .with_ymd_and_hms(2026, 1, 2, 3, 4, 5)
        .single()
        .unwrap_or_default();
    ExperimentStamp {
        creator: "00000000-0000-0000-0000-000000000001".to_owned(),
        created_at,
        expires_at: created_at + chrono::Duration::days(7),
    }
}

fn route_target() -> RouteTarget {
    RouteTarget {
        gateway_name: "insight".to_owned(),
        gateway_namespace: "insight-infra".to_owned(),
        gateway_section_name: String::new(),
        host: "preview.example.com".to_owned(),
        base_path: "/exp".to_owned(),
    }
}

fn rendered() -> Result<(Value, Value, Value), serde_json::Error> {
    let deployment = serde_json::to_value(objects::deployment(&name(), &tag(), &stamp()))?;
    let service = serde_json::to_value(objects::service(&name(), &tag(), &stamp()))?;
    let route = serde_json::to_value(objects::http_route(
        &name(),
        &tag(),
        &stamp(),
        &route_target(),
    ))?;
    Ok((deployment, service, route))
}

#[test]
fn resources_are_named_per_experiment() -> R {
    let (deployment, service, route) = rendered()?;

    for object in [&deployment, &service, &route] {
        assert_eq!(object["metadata"]["name"], "preview-widget-alpha");
    }
    Ok(())
}

#[test]
fn every_object_carries_the_experiment_label() -> R {
    let (deployment, service, route) = rendered()?;

    for object in [&deployment, &service, &route] {
        let labels = &object["metadata"]["labels"];
        assert_eq!(labels[objects::EXPERIMENT_LABEL], "widget-alpha");
        assert_eq!(labels["app.kubernetes.io/name"], "insight-preview");
        assert_eq!(labels["app.kubernetes.io/instance"], "preview-widget-alpha");
    }
    Ok(())
}

#[test]
fn httproute_prefix_strips_the_exp_path() -> R {
    let (_, _, route) = rendered()?;

    assert_eq!(route["apiVersion"], "gateway.networking.k8s.io/v1");
    assert_eq!(route["kind"], "HTTPRoute");
    assert_eq!(route["spec"]["hostnames"], json!(["preview.example.com"]));

    let parent = &route["spec"]["parentRefs"][0];
    assert_eq!(parent["name"], "insight");
    assert_eq!(parent["namespace"], "insight-infra");
    assert_eq!(
        parent.get("sectionName"),
        None,
        "empty sectionName must be omitted"
    );

    let rule = &route["spec"]["rules"][0];
    assert_eq!(
        rule["matches"][0]["path"],
        json!({"type": "PathPrefix", "value": "/exp/widget-alpha"})
    );
    assert_eq!(rule["filters"][0]["type"], "URLRewrite");
    assert_eq!(
        rule["filters"][0]["urlRewrite"]["path"],
        json!({"type": "ReplacePrefixMatch", "replacePrefixMatch": "/"})
    );
    assert_eq!(rule["backendRefs"][0]["name"], "preview-widget-alpha");
    assert_eq!(rule["backendRefs"][0]["port"], 80);
    Ok(())
}

#[test]
fn a_section_name_is_forwarded_when_configured() -> R {
    let mut target = route_target();
    target.gateway_section_name = "https".to_owned();

    let route = serde_json::to_value(objects::http_route(&name(), &tag(), &stamp(), &target))?;

    assert_eq!(route["spec"]["parentRefs"][0]["sectionName"], "https");
    Ok(())
}

#[test]
fn a_custom_base_path_is_honored() -> R {
    let mut target = route_target();
    target.base_path = "/preview".to_owned();

    let route = serde_json::to_value(objects::http_route(&name(), &tag(), &stamp(), &target))?;

    assert_eq!(
        route["spec"]["rules"][0]["matches"][0]["path"]["value"],
        "/preview/widget-alpha"
    );
    Ok(())
}

#[test]
fn service_selects_the_deployment_pods() -> R {
    let (deployment, service, _) = rendered()?;

    let selector = service["spec"]["selector"]
        .as_object()
        .ok_or("selector must be an object")?;
    let pod_labels = deployment["spec"]["template"]["metadata"]["labels"]
        .as_object()
        .ok_or("pod labels must be an object")?;

    for (key, value) in selector {
        assert_eq!(
            pod_labels.get(key),
            Some(value),
            "selector key {key} must be on the pods"
        );
    }
    assert_eq!(service["spec"]["type"], "ClusterIP");
    assert_eq!(
        service["spec"]["ports"][0],
        json!({"name": "http", "port": 80, "protocol": "TCP", "targetPort": "http"})
    );
    Ok(())
}

#[test]
fn the_deployment_selector_matches_its_pods() -> R {
    let (deployment, _, _) = rendered()?;

    assert_eq!(
        deployment["spec"]["selector"]["matchLabels"],
        deployment["spec"]["template"]["metadata"]["labels"]
    );
    Ok(())
}

#[test]
fn image_is_the_hardcoded_repository_at_the_pinned_tag() -> R {
    let (deployment, _, _) = rendered()?;

    let container = &deployment["spec"]["template"]["spec"]["containers"][0];
    assert_eq!(
        container["image"],
        format!("{}:preview-abc123", objects::IMAGE_REPOSITORY)
    );
    assert_eq!(container["imagePullPolicy"], "IfNotPresent");
    Ok(())
}

#[test]
fn the_pod_shape_is_fixed() -> R {
    let (deployment, _, _) = rendered()?;

    assert_eq!(deployment["spec"]["replicas"], 1);

    let pod = &deployment["spec"]["template"]["spec"];
    assert_eq!(pod["enableServiceLinks"], false);

    let container = &pod["containers"][0];
    assert_eq!(container["name"], "frontend");
    assert_eq!(
        container["ports"][0],
        json!({"name": "http", "containerPort": 8080, "protocol": "TCP"})
    );
    for probe in ["livenessProbe", "readinessProbe"] {
        assert_eq!(
            container[probe]["httpGet"]["path"], "/healthz",
            "for: {probe}"
        );
        assert_eq!(container[probe]["httpGet"]["port"], "http", "for: {probe}");
    }
    assert_eq!(
        container["resources"],
        json!({
            "requests": {"cpu": "50m", "memory": "64Mi"},
            "limits": {"cpu": "200m", "memory": "128Mi"}
        })
    );
    Ok(())
}

#[test]
fn the_frontend_carries_no_auth_env_and_no_command() -> R {
    let (deployment, _, _) = rendered()?;

    let container = &deployment["spec"]["template"]["spec"]["containers"][0];
    assert_eq!(container.get("env"), None, "no env injection surface");
    assert_eq!(
        container.get("command"),
        None,
        "no command injection surface"
    );
    assert_eq!(container.get("args"), None, "no args injection surface");
    Ok(())
}

#[test]
fn the_metadata_annotations_round_trip_into_a_record() -> R {
    let deployment = objects::deployment(&name(), &tag(), &stamp());

    let now = stamp().created_at;
    let record = objects::experiment_from_deployment(&deployment, now)
        .ok_or("a built deployment must read back as an experiment")?;

    assert_eq!(record.name, "widget-alpha");
    assert_eq!(record.tag, "preview-abc123");
    assert_eq!(record.creator, stamp().creator);
    assert_eq!(record.created_at, Some(stamp().created_at));
    assert_eq!(record.expires_at, Some(stamp().expires_at));
    Ok(())
}

#[test]
fn a_foreign_deployment_is_not_an_experiment() {
    let foreign = k8s_openapi::api::apps::v1::Deployment::default();

    assert!(objects::experiment_from_deployment(&foreign, Utc::now()).is_none());
}

#[test]
fn expired_experiments_do_not_count_against_the_cap() {
    let live = objects::deployment(&name(), &tag(), &stamp());
    let expired = {
        let past = stamp().created_at - chrono::Duration::days(30);
        objects::deployment(
            &name(),
            &tag(),
            &objects::ExperimentStamp {
                expires_at: past,
                ..stamp()
            },
        )
    };
    let foreign = k8s_openapi::api::apps::v1::Deployment::default();

    let now = stamp().created_at;
    let count = objects::live_experiment_count(&[live, expired, foreign], now);

    assert_eq!(count, 1, "only the unexpired experiment counts");
}
