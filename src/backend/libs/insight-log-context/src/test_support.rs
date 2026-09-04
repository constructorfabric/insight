//! Fires one request through a probe route wrapped in a [`LogContextLayer`]
//! and returns the captured JSON probe line plus its `log_ctx` span fields.

use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::routing::get;
use toolkit_security::SecurityContext;
use tower::ServiceExt;
use uuid::Uuid;

use crate::LogContextLayer;

pub const PROBE_MESSAGE: &str = "log context probe line";

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("probe request could not be built or served: {0}")]
    Request(String),
    #[error("runtime: {0}")]
    Runtime(#[from] io::Error),
    #[error("captured output is not JSON lines: {0}")]
    Json(#[from] serde_json::Error),
    #[error("no line carrying the probe message was captured")]
    ProbeLineMissing,
    #[error("the probe line carries no log_ctx span")]
    LogCtxSpanMissing,
}

#[derive(Clone, Debug, Default)]
struct SharedWriter(Arc<Mutex<Vec<u8>>>);

impl io::Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // SAFETY: a poisoned buffer is still appendable — the writer never leaves it torn.
        let mut inner = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        inner.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedWriter {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

async fn probe_handler() -> &'static str {
    tracing::info!("{PROBE_MESSAGE}");
    "ok"
}

/// # Errors
/// The probe line is missing from the captured output or carries no `log_ctx`
/// span — either means the layer is not doing its job.
pub fn capture_probe_line(
    layer: LogContextLayer,
    headers: &[(&str, &str)],
    tenant: Option<Uuid>,
) -> Result<(serde_json::Value, serde_json::Value), CaptureError> {
    let writer = SharedWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_writer(writer.clone())
        .with_max_level(tracing::Level::TRACE)
        .finish();

    let app = Router::new()
        .route("/probe", get(probe_handler))
        .layer(layer);

    let mut request = Request::builder().uri("/probe");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    if let Some(tenant_id) = tenant {
        let security = SecurityContext::builder()
            .subject_id(Uuid::from_u128(1))
            .subject_tenant_id(tenant_id)
            .build()
            .map_err(|e| CaptureError::Request(e.to_string()))?;
        request = request.extension(security);
    }
    let request = request
        .body(Body::empty())
        .map_err(|e| CaptureError::Request(e.to_string()))?;

    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    tracing::subscriber::with_default(subscriber, || {
        runtime
            .block_on(app.oneshot(request))
            .map_err(|e| CaptureError::Request(e.to_string()))
    })?;

    let captured = writer.0.lock().unwrap_or_else(PoisonError::into_inner);
    let lines = String::from_utf8_lossy(&captured)
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<Result<Vec<_>, _>>()?;

    let probe_line = lines
        .into_iter()
        .find(|line| {
            line.pointer("/fields/message")
                .and_then(serde_json::Value::as_str)
                == Some(PROBE_MESSAGE)
        })
        .ok_or(CaptureError::ProbeLineMissing)?;

    let log_ctx = probe_line
        .pointer("/spans")
        .and_then(serde_json::Value::as_array)
        .and_then(|spans| {
            spans.iter().find(|span| {
                span.get("name").and_then(serde_json::Value::as_str) == Some("log_ctx")
            })
        })
        .cloned()
        .ok_or(CaptureError::LogCtxSpanMissing)?;

    Ok((probe_line, log_ctx))
}
