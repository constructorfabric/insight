//! `log_ctx` span around each service's routes: every request-scoped line
//! carries `correlation_id` (echoed from the gateway's `X-Correlation-Id`,
//! `x-request-id` as fallback — never minted here), `tenant_id` when
//! authenticated, and `service` / `version`.

use std::sync::Arc;
use std::task::{Context, Poll};

use axum::http::Request;
use toolkit_security::SecurityContext;
use tower::{Layer, Service};
use tracing::field::Empty;
use tracing::instrument::{Instrument, Instrumented};

pub const CORRELATION_ID_HEADER: &str = "x-correlation-id";
pub const REQUEST_ID_HEADER: &str = "x-request-id";
pub const VERSION_ENV: &str = "INSIGHT_SERVICE_VERSION";

/// Built via [`service_identity!`] so the crate name and fallback version are
/// the calling service's. `INSIGHT_SERVICE_VERSION` (the image tag) wins.
#[derive(Debug)]
pub struct ServiceIdentity {
    name: &'static str,
    version: String,
}

impl ServiceIdentity {
    #[must_use]
    pub fn new(name: &'static str, crate_version: &str) -> Self {
        let version = std::env::var(VERSION_ENV).unwrap_or_else(|_| crate_version.to_owned());
        Self { name, version }
    }
}

#[macro_export]
macro_rules! service_identity {
    () => {
        $crate::ServiceIdentity::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
    };
}

#[derive(Debug, Clone)]
pub struct LogContextLayer {
    identity: Arc<ServiceIdentity>,
}

impl LogContextLayer {
    #[must_use]
    pub fn new(identity: ServiceIdentity) -> Self {
        Self {
            identity: Arc::new(identity),
        }
    }
}

impl<S> Layer<S> for LogContextLayer {
    type Service = LogContextService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        LogContextService {
            inner,
            identity: Arc::clone(&self.identity),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogContextService<S> {
    inner: S,
    identity: Arc<ServiceIdentity>,
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
            service = self.identity.name,
            version = %self.identity.version,
            correlation_id = %correlation_id,
            tenant_id = Empty,
        );
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
