//! Axum middleware emitting `http.server.request.duration` (seconds) with
//! `http.request.method`, `http.response.status_code` and `http.route`, per
//! `OTel` HTTP server semconv.
//!
//! INVARIANT: instruments record into the global meter provider, so this
//! crate's `opentelemetry` major must match the toolkit's — a different
//! major is a different global, and recording silently no-ops.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use axum::extract::MatchedPath;
use axum::http::Request;
use axum::response::Response;
use opentelemetry::metrics::{Histogram, Meter};
use opentelemetry::{KeyValue, global};
use tower::{Layer, Service};

// Same buckets as the toolkit's client layer, so the two sides' percentiles
// stay comparable.
const DURATION_BOUNDARIES_SECS: &[f64] = &[
    0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 150.0, 300.0, 600.0,
];

/// Unknown methods collapse to `_OTHER` to bound attribute cardinality.
fn normalize_method(method: &axum::http::Method) -> &'static str {
    use axum::http::Method;
    match *method {
        Method::GET => "GET",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::DELETE => "DELETE",
        Method::PATCH => "PATCH",
        Method::HEAD => "HEAD",
        Method::OPTIONS => "OPTIONS",
        Method::CONNECT => "CONNECT",
        Method::TRACE => "TRACE",
        _ => "_OTHER",
    }
}

/// Records `http.server.request.duration` for every request on the router
/// it wraps.
#[derive(Clone)]
pub struct ServerMetricsLayer {
    duration: Histogram<f64>,
}

impl ServerMetricsLayer {
    /// Registers against the global meter; `gear_name` is the
    /// instrumentation scope.
    #[must_use]
    pub fn new(gear_name: &str) -> Self {
        let scope = opentelemetry::InstrumentationScope::builder(gear_name.to_owned()).build();
        let meter = global::meter_with_scope(scope);
        Self::with_meter(&meter)
    }

    #[must_use]
    pub fn with_meter(meter: &Meter) -> Self {
        let duration = meter
            .f64_histogram("http.server.request.duration")
            .with_description("Duration of inbound HTTP server requests")
            .with_unit("s")
            .with_boundaries(DURATION_BOUNDARIES_SECS.to_vec())
            .build();
        Self { duration }
    }
}

impl<S> Layer<S> for ServerMetricsLayer {
    type Service = ServerMetricsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ServerMetricsService {
            inner,
            duration: self.duration.clone(),
        }
    }
}

#[derive(Clone)]
pub struct ServerMetricsService<S> {
    inner: S,
    duration: Histogram<f64>,
}

impl<S, ReqBody> Service<Request<ReqBody>> for ServerMetricsService<S>
where
    S: Service<Request<ReqBody>, Response = Response> + Clone + Send + 'static,
    S::Future: Send,
    ReqBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        // INVARIANT: the label is the matched route template, never the raw
        // path — raw paths make the label set unbounded.
        let route = req
            .extensions()
            .get::<MatchedPath>()
            .map(|matched| matched.as_str().to_owned());
        let method = normalize_method(req.method());
        let duration = self.duration.clone();

        // INVARIANT: call the instance that was poll_ready'd (Tower contract).
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(async move {
            let start = Instant::now();
            let result = inner.call(req).await;
            let elapsed = start.elapsed().as_secs_f64();

            if let Ok(response) = &result {
                let mut attrs = vec![
                    KeyValue::new("http.request.method", method),
                    KeyValue::new(
                        "http.response.status_code",
                        i64::from(response.status().as_u16()),
                    ),
                ];
                if let Some(route) = route {
                    attrs.push(KeyValue::new("http.route", route));
                }
                duration.record(elapsed, &attrs);
            }

            result
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::StatusCode;
    use axum::routing::get;
    use opentelemetry::metrics::MeterProvider;
    use opentelemetry_sdk::metrics::data::{AggregatedMetrics, HistogramDataPoint, MetricData};
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};
    use tower::ServiceExt;

    type R = Result<(), Box<dyn std::error::Error>>;

    fn test_provider() -> (SdkMeterProvider, InMemoryMetricExporter) {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter.clone())
            .build();
        (provider, exporter)
    }

    fn find_duration_point(
        exporter: &InMemoryMetricExporter,
        expected: &[(&str, &str)],
    ) -> Option<HistogramDataPoint<f64>> {
        let batches = exporter.get_finished_metrics().ok()?;
        for rm in &batches {
            for sm in rm.scope_metrics() {
                for metric in sm.metrics() {
                    if metric.name() != "http.server.request.duration" {
                        continue;
                    }
                    let AggregatedMetrics::F64(MetricData::Histogram(hist)) = metric.data() else {
                        continue;
                    };
                    for dp in hist.data_points() {
                        let matches = expected.iter().all(|(k, v)| {
                            dp.attributes()
                                .any(|kv| kv.key.as_str() == *k && kv.value.to_string() == *v)
                        });
                        if matches {
                            return Some(dp.clone());
                        }
                    }
                }
            }
        }
        None
    }

    #[tokio::test]
    async fn records_route_template_not_raw_path() -> R {
        let (provider, exporter) = test_provider();
        let layer = ServerMetricsLayer::with_meter(&provider.meter("test"));

        let app = Router::new()
            .route("/users/{id}", get(async || StatusCode::OK))
            .layer(layer);
        let req = Request::builder()
            .method(axum::http::Method::GET)
            .uri("/users/123")
            .body(Body::empty())?;

        let resp = app.oneshot(req).await?;
        assert_eq!(resp.status(), StatusCode::OK);

        provider.force_flush()?;
        let point = find_duration_point(
            &exporter,
            &[
                ("http.request.method", "GET"),
                ("http.route", "/users/{id}"),
                ("http.response.status_code", "200"),
            ],
        )
        .ok_or("a data point with the route template should be exported")?;
        assert_eq!(point.count(), 1, "exactly one observation recorded");
        assert!(
            point
                .attributes()
                .all(|kv| kv.value.to_string() != "/users/123"),
            "raw path must never become a label"
        );
        Ok(())
    }

    #[tokio::test]
    async fn records_error_status_codes() -> R {
        let (provider, exporter) = test_provider();
        let layer = ServerMetricsLayer::with_meter(&provider.meter("test"));

        let app = Router::new()
            .route("/boom", get(async || StatusCode::INTERNAL_SERVER_ERROR))
            .layer(layer);
        let req = Request::builder().uri("/boom").body(Body::empty())?;

        app.oneshot(req).await?;

        provider.force_flush()?;
        let point = find_duration_point(
            &exporter,
            &[
                ("http.route", "/boom"),
                ("http.response.status_code", "500"),
            ],
        )
        .ok_or("a data point for the 5xx response should be exported")?;
        assert_eq!(point.count(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn unmatched_request_omits_route_attribute() -> R {
        let (provider, exporter) = test_provider();
        let layer = ServerMetricsLayer::with_meter(&provider.meter("test"));

        let app = Router::new()
            .route("/known", get(async || StatusCode::OK))
            .layer(layer);
        let req = Request::builder().uri("/nope").body(Body::empty())?;

        let resp = app.oneshot(req).await?;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        provider.force_flush()?;
        let point = find_duration_point(&exporter, &[("http.response.status_code", "404")])
            .ok_or("the 404 should still be observed")?;
        assert!(
            point.attributes().all(|kv| kv.key.as_str() != "http.route"),
            "no matched route means no http.route label"
        );
        Ok(())
    }

    #[test]
    fn normalize_method_caps_unknown() {
        assert_eq!(normalize_method(&axum::http::Method::GET), "GET");
        let custom = axum::http::Method::from_bytes(b"PROPFIND").ok();
        assert_eq!(
            custom.as_ref().map(normalize_method),
            Some("_OTHER"),
            "unknown verbs must collapse to _OTHER"
        );
    }
}
