//! Meter-provider lifecycle for the seed/sync CLI jobs.
//!
//! The toolkit installs a global meter provider inside `run_server`, but the
//! `seed` and `sync` subcommands run outside it — so their domain instruments
//! ([`crate::infra::metrics`]) would record into the no-op global. This builds
//! a local `SdkMeterProvider` from the same `opentelemetry` config the server
//! reads, registers it globally, and flushes it on [`MetricsGuard::shutdown`].
//!
//! A CLI job exits long before the periodic exporter's interval elapses, so the
//! final `force_flush` is what actually delivers a run's series — without it
//! nothing reaches the collector.

use opentelemetry::KeyValue;
use opentelemetry_otlp::{MetricExporter, Protocol, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use toolkit::telemetry::config::{Exporter, ExporterKind, OpenTelemetryConfig};

const DEFAULT_GRPC_ENDPOINT: &str = "http://127.0.0.1:4317";
const DEFAULT_HTTP_ENDPOINT: &str = "http://127.0.0.1:4318";

/// A registered local meter provider. Hold it for the length of a job and call
/// [`MetricsGuard::shutdown`] before exit to flush the run's metrics.
#[must_use = "hold the guard for the job's lifetime and call shutdown() to flush"]
pub(crate) struct MetricsGuard {
    provider: Option<SdkMeterProvider>,
}

impl MetricsGuard {
    /// Build and globally register a meter provider from `cfg`. A no-op that
    /// yields an empty guard when metrics export is disabled, or when the
    /// exporter cannot be built (logged, never fatal — a job must still run).
    pub(crate) fn install(cfg: &OpenTelemetryConfig) -> Self {
        if !cfg.metrics.enabled {
            tracing::info!("OpenTelemetry metrics disabled; seed instruments are no-ops");
            return Self { provider: None };
        }

        match build_provider(cfg) {
            Ok(provider) => {
                opentelemetry::global::set_meter_provider(provider.clone());
                tracing::info!("seed metrics provider installed");
                Self {
                    provider: Some(provider),
                }
            }
            Err(error) => {
                tracing::error!(error = %error, "seed metrics provider not installed");
                Self { provider: None }
            }
        }
    }

    /// Flush and shut the provider down. Idempotent and best-effort: a failed
    /// flush is logged, never propagated — losing a metric must not fail a job.
    pub(crate) fn shutdown(mut self) {
        let Some(provider) = self.provider.take() else {
            return;
        };

        if let Err(error) = provider.force_flush() {
            tracing::warn!(error = %error, "seed metrics force_flush failed");
        }
        if let Err(error) = provider.shutdown() {
            tracing::warn!(error = %error, "seed metrics shutdown failed");
        }
    }
}

fn build_provider(cfg: &OpenTelemetryConfig) -> anyhow::Result<SdkMeterProvider> {
    let exporter = build_exporter(cfg.metrics_exporter())?;
    let resource = build_resource(cfg);

    let mut builder = SdkMeterProvider::builder()
        .with_periodic_exporter(exporter)
        .with_resource(resource);
    if let Some(limit) = cfg.metrics.cardinality_limit {
        builder = builder.with_view(move |_: &opentelemetry_sdk::metrics::Instrument| {
            opentelemetry_sdk::metrics::Stream::builder()
                .with_cardinality_limit(limit)
                .build()
                .ok()
        });
    }

    Ok(builder.build())
}

fn build_exporter(exporter: Option<&Exporter>) -> anyhow::Result<MetricExporter> {
    let kind = exporter.map_or(ExporterKind::OtlpGrpc, |e| e.kind);
    let endpoint = exporter
        .and_then(|e| e.endpoint.clone())
        .unwrap_or_else(|| default_endpoint(kind).to_owned());
    let timeout = exporter
        .and_then(|e| e.timeout_ms)
        .map(std::time::Duration::from_millis);

    let built = match kind {
        ExporterKind::OtlpHttp => {
            let mut b = MetricExporter::builder()
                .with_http()
                .with_protocol(Protocol::HttpBinary)
                .with_endpoint(&endpoint);
            if let Some(t) = timeout {
                b = b.with_timeout(t);
            }
            b.build()?
        }
        ExporterKind::OtlpGrpc => {
            let mut b = MetricExporter::builder()
                .with_tonic()
                .with_endpoint(&endpoint);
            if let Some(t) = timeout {
                b = b.with_timeout(t);
            }
            b.build()?
        }
    };

    Ok(built)
}

fn build_resource(cfg: &OpenTelemetryConfig) -> Resource {
    let mut attrs = vec![KeyValue::new(
        "service.name",
        cfg.resource.service_name.clone(),
    )];
    for (key, value) in &cfg.resource.attributes {
        if key == "service.name" {
            continue;
        }
        attrs.push(KeyValue::new(key.clone(), value.clone()));
    }

    Resource::builder_empty().with_attributes(attrs).build()
}

const fn default_endpoint(kind: ExporterKind) -> &'static str {
    match kind {
        ExporterKind::OtlpHttp => DEFAULT_HTTP_ENDPOINT,
        ExporterKind::OtlpGrpc => DEFAULT_GRPC_ENDPOINT,
    }
}
