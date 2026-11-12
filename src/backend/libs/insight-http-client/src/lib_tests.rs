use super::{client_builder, client_builder_with_extra_roots};

// Self-signed test CAs (openssl, not trusted anywhere). Only parsed.
const TEST_CA: &[u8] = include_bytes!("test-ca.pem");
const TEST_CA_BUNDLE: &[u8] = include_bytes!("test-ca-bundle.pem");

#[test]
fn default_builder_builds_a_client() -> anyhow::Result<()> {
    client_builder()?.build()?;
    Ok(())
}

#[test]
fn extra_roots_accept_a_single_ca_and_a_bundle() -> anyhow::Result<()> {
    client_builder_with_extra_roots(TEST_CA)?.build()?;
    client_builder_with_extra_roots(TEST_CA_BUNDLE)?.build()?;
    Ok(())
}

#[test]
fn extra_roots_reject_empty_and_garbage_bundles() {
    assert!(client_builder_with_extra_roots(b"").is_err());
    assert!(client_builder_with_extra_roots(b"# comment only\n").is_err());
    let truncated = &TEST_CA[..TEST_CA.len() / 2];
    assert!(client_builder_with_extra_roots(truncated).is_err());
}
