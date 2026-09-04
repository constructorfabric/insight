use insight_log_context::CORRELATION_ID_HEADER;
use insight_log_context::test_support::capture_probe_line;
use uuid::Uuid;

type R = Result<(), Box<dyn std::error::Error>>;

#[test]
fn request_lines_carry_the_service_identity_and_echo_the_arriving_id() -> R {
    let tenant = Uuid::from_u128(7);

    let (_, log_ctx) = capture_probe_line(
        super::log_context_layer(),
        &[(CORRELATION_ID_HEADER, "corr-3190")],
        Some(tenant),
    )?;

    assert_eq!(log_ctx["service"], "identity-resolution");
    assert_eq!(log_ctx["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(log_ctx["correlation_id"], "corr-3190");
    assert_eq!(log_ctx["tenant_id"], tenant.to_string());
    Ok(())
}
