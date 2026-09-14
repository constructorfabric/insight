//! Seeded-leak checks (insight#2488 AC-4): previews holds no store credentials
//! and no person records, so its logging path is the request itself — a seeded
//! bearer token and session cookie must never surface in what a request logs.

use insight_log_context::test_support::capture_probe_output;
use insight_log_context::{CORRELATION_ID_HEADER, LogContextLayer};
use uuid::Uuid;

type R = Result<(), Box<dyn std::error::Error>>;

const SEEDED_BEARER: &str = "seeded-bearer-3192";
const SEEDED_SESSION_COOKIE: &str = "seeded-session-cookie-3192";

#[test]
fn a_request_carrying_credentials_logs_none_of_them() -> R {
    let output = capture_probe_output(
        LogContextLayer::new(),
        &[
            (CORRELATION_ID_HEADER, "corr-3192"),
            ("authorization", &format!("Bearer {SEEDED_BEARER}")),
            ("cookie", &format!("__Host-sid={SEEDED_SESSION_COOKIE}")),
        ],
        Some(Uuid::from_u128(7)),
    )?;

    assert!(!output.is_empty(), "the probe request must log something");
    assert!(
        !output.contains(SEEDED_BEARER),
        "leaked: bearer in {output}"
    );
    assert!(
        !output.contains(SEEDED_SESSION_COOKIE),
        "leaked: session cookie in {output}"
    );
    Ok(())
}
