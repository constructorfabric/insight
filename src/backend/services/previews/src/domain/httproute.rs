//! A minimal typed Gateway API `HTTPRoute` (gateway.networking.k8s.io/v1).
//!
//! k8s-openapi carries no Gateway API types, so this defines exactly the
//! subset a preview experiment's route uses — parentRefs, hostnames, one
//! `PathPrefix` rule with a `URLRewrite` prefix-strip and one `backendRef` — with a
//! manual [`kube::Resource`] impl so `Api::<HttpRoute>::namespaced` works.

use std::borrow::Cow;

use k8s_openapi::NamespaceResourceScope;
use kube::api::ObjectMeta;
use serde::{Deserialize, Serialize};

pub const API_VERSION: &str = "gateway.networking.k8s.io/v1";
pub const KIND: &str = "HTTPRoute";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpRoute {
    /// Constant [`API_VERSION`]/[`KIND`]; carried as fields because the API
    /// server requires them in a create body and serde has nowhere else to
    /// put them.
    pub api_version: String,
    pub kind: String,
    pub metadata: ObjectMeta,
    pub spec: HttpRouteSpec,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpRouteSpec {
    pub parent_refs: Vec<ParentRef>,
    pub hostnames: Vec<String>,
    pub rules: Vec<HttpRouteRule>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParentRef {
    pub name: String,
    pub namespace: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section_name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpRouteRule {
    pub matches: Vec<RouteMatch>,
    pub filters: Vec<RouteFilter>,
    pub backend_refs: Vec<BackendRef>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteMatch {
    pub path: PathMatch,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathMatch {
    pub r#type: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteFilter {
    pub r#type: String,
    pub url_rewrite: UrlRewrite,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlRewrite {
    pub path: RewritePath,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RewritePath {
    pub r#type: String,
    pub replace_prefix_match: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendRef {
    pub name: String,
    pub port: i32,
}

impl kube::Resource for HttpRoute {
    type DynamicType = ();
    type Scope = NamespaceResourceScope;

    fn kind((): &()) -> Cow<'static, str> {
        KIND.into()
    }

    fn group((): &()) -> Cow<'static, str> {
        "gateway.networking.k8s.io".into()
    }

    fn version((): &()) -> Cow<'static, str> {
        "v1".into()
    }

    fn plural((): &()) -> Cow<'static, str> {
        "httproutes".into()
    }

    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }

    fn meta_mut(&mut self) -> &mut ObjectMeta {
        &mut self.metadata
    }
}
