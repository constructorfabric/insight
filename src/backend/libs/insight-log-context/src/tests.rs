use uuid::Uuid;

use crate::test_support::capture_probe_line;
use crate::{CORRELATION_ID_HEADER, LogContextLayer, REQUEST_ID_HEADER, ServiceIdentity};

type R = Result<(), Box<dyn std::error::Error>>;

fn layer() -> LogContextLayer {
    LogContextLayer::new(ServiceIdentity::new("probe-service", "9.9.9"))
}

fn field<'a>(span: &'a serde_json::Value, name: &str) -> Option<&'a str> {
    span.get(name).and_then(serde_json::Value::as_str)
}

#[test]
fn a_request_line_carries_service_version_tenant_and_correlation_id() -> R {
    let tenant = Uuid::from_u128(42);

    let (_, log_ctx) = capture_probe_line(
        layer(),
        &[(CORRELATION_ID_HEADER, "corr-under-test")],
        Some(tenant),
    )?;

    assert_eq!(field(&log_ctx, "service"), Some("probe-service"));
    assert_eq!(field(&log_ctx, "version"), Some("9.9.9"));
    assert_eq!(field(&log_ctx, "correlation_id"), Some("corr-under-test"));
    assert_eq!(
        field(&log_ctx, "tenant_id"),
        Some(tenant.to_string().as_str())
    );
    Ok(())
}

#[test]
fn the_correlation_id_is_echoed_not_minted() -> R {
    let (_, first) = capture_probe_line(layer(), &[(CORRELATION_ID_HEADER, "echo-me-1")], None)?;
    let (_, second) = capture_probe_line(layer(), &[(CORRELATION_ID_HEADER, "echo-me-2")], None)?;

    assert_eq!(field(&first, "correlation_id"), Some("echo-me-1"));
    assert_eq!(field(&second, "correlation_id"), Some("echo-me-2"));
    Ok(())
}

#[test]
fn without_a_gateway_id_the_request_id_is_the_fallback() -> R {
    let (_, log_ctx) = capture_probe_line(layer(), &[(REQUEST_ID_HEADER, "rid-fallback")], None)?;

    assert_eq!(field(&log_ctx, "correlation_id"), Some("rid-fallback"));
    Ok(())
}

#[test]
fn the_gateway_id_wins_over_the_request_id() -> R {
    let (_, log_ctx) = capture_probe_line(
        layer(),
        &[
            (CORRELATION_ID_HEADER, "corr-wins"),
            (REQUEST_ID_HEADER, "rid-loses"),
        ],
        None,
    )?;

    assert_eq!(field(&log_ctx, "correlation_id"), Some("corr-wins"));
    Ok(())
}

#[test]
fn without_a_security_context_the_tenant_field_is_absent() -> R {
    let (_, log_ctx) = capture_probe_line(layer(), &[(CORRELATION_ID_HEADER, "corr")], None)?;

    assert!(
        log_ctx.get("tenant_id").is_none(),
        "unauthenticated request grew a tenant: {log_ctx}"
    );
    Ok(())
}
