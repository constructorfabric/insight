//! Seeded-leak checks (insight#2488 AC-4): write the proxy token and a vendor
//! git credential through the logging path — each must render as a redaction
//! marker, never as its value.

use insight_log_context::test_support::capture_output;

use crate::config::GearConfig;
use crate::engine::runner::GitCredentials;

const SEEDED_PROXY_TOKEN: &str = "seeded-proxy-token-3192";
const SEEDED_GIT_TOKEN: &str = "seeded-git-token-3192";

#[test]
fn the_boot_config_line_renders_a_marker_never_the_proxy_token() {
    let config = GearConfig {
        proxy_token: SEEDED_PROXY_TOKEN.to_owned(),
        ..GearConfig::default()
    };

    let output = capture_output(|| tracing::info!(config = ?config, "starting git-cli-proxy gear"));

    assert!(
        !output.contains(SEEDED_PROXY_TOKEN),
        "leaked: proxy_token in {output}"
    );
    assert!(output.contains("<redacted>"), "no marker in {output}");
}

#[test]
fn logged_git_credentials_render_a_marker_never_the_token() {
    let credentials = GitCredentials {
        username: "ci-bot".to_owned(),
        token: SEEDED_GIT_TOKEN.to_owned(),
    };

    let output =
        capture_output(|| tracing::error!(credentials = ?credentials, "seeded leak probe"));

    assert!(
        !output.contains(SEEDED_GIT_TOKEN),
        "leaked: git token in {output}"
    );
    assert!(output.contains("<redacted>"), "no marker in {output}");
}
