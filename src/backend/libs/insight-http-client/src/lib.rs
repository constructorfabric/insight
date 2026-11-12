//! Outbound HTTP clients on rustls with the platform's crypto provider.
//!
//! Every production `reqwest::Client` starts from [`client_builder`] (or
//! [`client_builder_with_extra_roots`]). The TLS config is built explicitly
//! from `toolkit_security::tls::configured_provider()` and handed to reqwest
//! with `use_preconfigured_tls`, so the backend does not depend on which
//! reqwest TLS features Cargo unified for the binary.
//!
//! Trust anchors are the system store (`/etc/ssl/certs` in the runtime
//! images), the same store the previous native-tls backend used on Linux.

use std::sync::Arc;

use anyhow::Context as _;
use rustls::RootCertStore;
use rustls_pki_types::CertificateDer;
use rustls_pki_types::pem::PemObject as _;

/// A `reqwest::ClientBuilder` whose TLS runs on the configured provider and
/// trusts the system root store.
///
/// # Errors
/// Fails when the provider rejects the default protocol versions.
pub fn client_builder() -> anyhow::Result<reqwest::ClientBuilder> {
    builder_with_roots(system_roots())
}

/// Like [`client_builder`], also trusting every certificate in `pem` (a PEM
/// bundle: a root, or an intermediate plus root).
///
/// # Errors
/// Fails when `pem` holds no certificate, a certificate does not parse, or
/// the provider rejects the default protocol versions.
pub fn client_builder_with_extra_roots(pem: &[u8]) -> anyhow::Result<reqwest::ClientBuilder> {
    let mut roots = system_roots();
    let mut added = 0_usize;
    for cert in CertificateDer::pem_slice_iter(pem) {
        let cert = cert.context("parse PEM certificate")?;
        roots.add(cert).context("add extra root certificate")?;
        added += 1;
    }
    anyhow::ensure!(added > 0, "PEM bundle contained no certificates");
    builder_with_roots(roots)
}

fn builder_with_roots(roots: RootCertStore) -> anyhow::Result<reqwest::ClientBuilder> {
    let provider = Arc::new(toolkit_security::tls::configured_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .context("TLS provider rejects the default protocol versions")?
        .with_root_certificates(roots)
        .with_no_client_auth();
    // reqwest leaves ALPN to the caller with a preconfigured config.
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(reqwest::Client::builder().use_preconfigured_tls(config))
}

/// The system trust store. Unreadable entries are skipped; an empty store
/// is logged rather than fatal, since plain-HTTP clients need no anchors.
fn system_roots() -> RootCertStore {
    let loaded = rustls_native_certs::load_native_certs();
    for error in &loaded.errors {
        tracing::warn!(error = %error, "skipping unreadable system certificate");
    }
    let mut roots = RootCertStore::empty();
    let (added, ignored) = roots.add_parsable_certificates(loaded.certs);
    if ignored > 0 {
        tracing::warn!(ignored, "skipped unparsable system certificates");
    }
    if added == 0 {
        tracing::warn!("system trust store is empty; HTTPS requests will fail verification");
    }
    roots
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
