//! `log_ctx` span around each service's routes: every request-scoped line
//! carries `correlation_id` (echoed from the gateway's `X-Correlation-Id`,
//! `x-request-id` as fallback — never minted here), `tenant_id` when
//! authenticated, and the `service` / `version` set once at startup from the
//! config's `opentelemetry.resource` (see [`init_identity`]).

use std::sync::OnceLock;
use std::task::{Context, Poll};

use axum::http::Request;
use toolkit_security::SecurityContext;
use tower::{Layer, Service};
use tracing::field::Empty;
use tracing::instrument::{Instrument, Instrumented};

pub const CORRELATION_ID_HEADER: &str = "x-correlation-id";
pub const REQUEST_ID_HEADER: &str = "x-request-id";
pub const VERSION_ATTRIBUTE: &str = "service.version";

#[derive(Debug)]
struct ServiceIdentity {
    service: String,
    version: Option<String>,
}

static IDENTITY: OnceLock<ServiceIdentity> = OnceLock::new();

/// [`init_identity`] from the config's `opentelemetry.resource` fields.
pub fn init_identity_from_resource(
    service_name: &str,
    attributes: &std::collections::BTreeMap<String, String>,
) {
    init_identity(
        service_name,
        attributes.get(VERSION_ATTRIBUTE).map(String::as_str),
    );
}

/// Set once from `main`, before the server starts. Later calls are ignored.
pub fn init_identity(service: &str, version: Option<&str>) {
    let _ = IDENTITY.set(ServiceIdentity {
        service: service.to_owned(),
        version: version.map(str::to_owned),
    });
}

#[derive(Debug, Clone, Default)]
pub struct LogContextLayer;

impl LogContextLayer {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl<S> Layer<S> for LogContextLayer {
    type Service = LogContextService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        LogContextService { inner }
    }
}

#[derive(Debug, Clone)]
pub struct LogContextService<S> {
    inner: S,
}

impl<S, B> Service<Request<B>> for LogContextService<S>
where
    S: Service<Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Instrumented<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        let header = |name: &str| {
            req.headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
        };
        let correlation_id = header(CORRELATION_ID_HEADER)
            .or_else(|| header(REQUEST_ID_HEADER))
            .unwrap_or_default();

        let span = tracing::info_span!(
            "log_ctx",
            service = Empty,
            version = Empty,
            correlation_id = %correlation_id,
            tenant_id = Empty,
        );
        if let Some(identity) = IDENTITY.get() {
            span.record("service", identity.service.as_str());
            if let Some(version) = &identity.version {
                span.record("version", version.as_str());
            }
        }
        if let Some(security) = req.extensions().get::<SecurityContext>() {
            span.record(
                "tenant_id",
                tracing::field::display(security.subject_tenant_id()),
            );
        }

        self.inner.call(req).instrument(span)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
mod tests;
