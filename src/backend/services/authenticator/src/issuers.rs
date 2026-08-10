//! Host-keyed issuer selection (ADR-0003): the request `Host` picks the
//! broker realm, but only where a flow STARTS (`/auth/login`); everywhere
//! else (callback, refresh, logout) the issuer is already pinned server-side
//! and Host is never re-consulted. Empty `idp.hosts` = single-issuer mode,
//! matching every host; in map mode an unmatched host fails closed.

use std::collections::HashMap;
use std::sync::Arc;

use crate::config::AuthenticatorConfig;
use crate::oidc::OidcClient;

/// The configured issuers, keyed by normalized host and by issuer URL.
pub struct IssuerSelector {
    by_host: HashMap<String, Arc<OidcClient>>,
    by_issuer: HashMap<String, Arc<OidcClient>>,
    /// Single-issuer (degenerate) mode: this client matches every host.
    any_host: Option<Arc<OidcClient>>,
}

impl IssuerSelector {
    /// Build every issuer's client up front (fail closed at boot, not on the
    /// first login), sharing one HTTP connection pool.
    ///
    /// # Errors
    /// Fails on an unbuildable HTTP client (bad `extra_ca_cert_path`), two
    /// host keys normalizing to the same host, or one issuer URL appearing
    /// under two hosts (sessions pin the issuer alone, so it must resolve to
    /// exactly one client registration).
    pub fn build(cfg: &AuthenticatorConfig) -> anyhow::Result<Self> {
        let idp = &cfg.idp;
        let http = crate::oidc::build_http(idp)?;

        if idp.hosts.is_empty() {
            let client = Arc::new(OidcClient::flat(idp, &cfg.redirect_uri, http));
            return Ok(Self {
                by_host: HashMap::new(),
                by_issuer: HashMap::from([(idp.issuer_url.clone(), client.clone())]),
                any_host: Some(client),
            });
        }

        let mut by_host = HashMap::new();
        let mut by_issuer: HashMap<String, Arc<OidcClient>> = HashMap::new();
        for (host, entry) in &idp.hosts {
            let client = Arc::new(OidcClient::for_host(
                idp,
                entry,
                &cfg.redirect_uri,
                http.clone(),
            ));
            let key = normalize_host(host);
            anyhow::ensure!(
                by_host.insert(key.clone(), client.clone()).is_none(),
                "idp.hosts: two keys normalize to the same host {key:?}"
            );
            anyhow::ensure!(
                by_issuer.insert(entry.issuer_url.clone(), client).is_none(),
                "idp.hosts: issuer {:?} appears under two hosts — an issuer must map \
                 to exactly one client registration (sessions resolve by issuer alone)",
                entry.issuer_url
            );
        }
        Ok(Self {
            by_host,
            by_issuer,
            any_host: None,
        })
    }

    /// Select the issuer for a flow-starting request by its `Host` header.
    /// `None` = no configured issuer serves this host: reject fail closed.
    #[must_use]
    pub fn for_host(&self, host: &str) -> Option<Arc<OidcClient>> {
        if let Some(single) = &self.any_host {
            return Some(single.clone());
        }
        self.by_host.get(&normalize_host(host)).cloned()
    }

    /// Resolve a server-side pinned issuer (session record / validated token).
    #[must_use]
    pub fn for_issuer(&self, issuer: &str) -> Option<Arc<OidcClient>> {
        self.by_issuer.get(issuer).cloned()
    }

    /// Like [`Self::for_issuer`], tolerating the empty issuer that state
    /// written by a pre-map replica carries: in single-issuer mode it can
    /// only mean the one client; in map mode it stays unresolvable.
    #[must_use]
    pub fn for_stored_issuer(&self, issuer: &str) -> Option<Arc<OidcClient>> {
        if issuer.is_empty() {
            return self.any_host.clone();
        }
        self.for_issuer(issuer)
    }
}

/// Canonical form of a request `Host` / map key for the exact-match lookup:
/// lowercased, port stripped (`:8443`; bracketed IPv6 literals keep their
/// colons), trailing FQDN dot dropped. Anything beyond that (scheme, path,
/// userinfo) is not a hostname and simply won't match a validated map key.
#[must_use]
pub fn normalize_host(raw: &str) -> String {
    let host = raw.trim().to_ascii_lowercase();
    let host = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(rest).to_owned(),
        None => match host.rsplit_once(':') {
            Some((name, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
                name.to_owned()
            }
            _ => host,
        },
    };
    host.trim_end_matches('.').to_owned()
}

/// Peek a `logout_token`'s `iss` claim WITHOUT verification — used only to
/// select the issuer whose JWKS/claims checks then validate the token in
/// full, so a forged `iss` buys nothing but a verification failure.
#[must_use]
pub fn unverified_issuer(jwt: &str) -> Option<String> {
    crate::oidc::payload_string(jwt, "iss")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::config::{HostIdpConfig, IdpConfig};

    fn cfg_with_hosts(hosts: &[(&str, &str)]) -> AuthenticatorConfig {
        AuthenticatorConfig {
            redirect_uri: "https://gw.example/auth/callback".to_owned(),
            idp: IdpConfig {
                issuer_url: "https://idp.example".to_owned(),
                client_id: "flat-client".to_owned(),
                hosts: hosts
                    .iter()
                    .map(|(host, issuer)| {
                        (
                            (*host).to_owned(),
                            HostIdpConfig {
                                issuer_url: (*issuer).to_owned(),
                                client_id: format!("client-{issuer}"),
                                ..HostIdpConfig::default()
                            },
                        )
                    })
                    .collect(),
                ..IdpConfig::default()
            },
            ..AuthenticatorConfig::default()
        }
    }

    #[test]
    fn empty_map_is_single_issuer_mode_matching_any_host() {
        let selector = IssuerSelector::build(&cfg_with_hosts(&[])).unwrap();
        for host in ["a.example", "b.example:8443", "ANYTHING", ""] {
            let client = selector.for_host(host).expect("degenerate matches all");
            assert_eq!(client.issuer(), "https://idp.example", "host {host:?}");
        }
        assert!(selector.for_issuer("https://idp.example").is_some());
        assert!(selector.for_stored_issuer("").is_some());
    }

    #[test]
    fn multi_entry_map_selects_by_exact_host_and_fails_closed() {
        let selector = IssuerSelector::build(&cfg_with_hosts(&[
            ("a.example", "https://kc.example/realms/a"),
            ("b.example", "https://kc.example/realms/b"),
        ]))
        .unwrap();

        let a = selector
            .for_host("a.example")
            .expect("a.example configured");
        assert_eq!(a.issuer(), "https://kc.example/realms/a");
        let b = selector
            .for_host("b.example")
            .expect("b.example configured");
        assert_eq!(b.issuer(), "https://kc.example/realms/b");

        for unknown in ["c.example", "a.example.evil", "example", ""] {
            assert!(
                selector.for_host(unknown).is_none(),
                "should fail closed: {unknown:?}"
            );
        }
        // The flat idp.* fields take no part in selection in map mode.
        assert!(selector.for_issuer("https://idp.example").is_none());
        assert!(selector.for_stored_issuer("").is_none());
        assert!(
            selector
                .for_stored_issuer("https://kc.example/realms/a")
                .is_some()
        );
    }

    #[test]
    fn host_matching_normalizes_case_and_port() {
        let selector = IssuerSelector::build(&cfg_with_hosts(&[(
            "a.example",
            "https://kc.example/realms/a",
        )]))
        .unwrap();
        for host in ["A.Example", "a.example:443", "a.example.", "A.EXAMPLE:8443"] {
            assert!(selector.for_host(host).is_some(), "should match: {host:?}");
        }
    }

    #[test]
    fn duplicate_issuer_across_hosts_is_rejected() {
        let cfg = cfg_with_hosts(&[
            ("a.example", "https://kc.example/realms/shared"),
            ("b.example", "https://kc.example/realms/shared"),
        ]);
        assert!(IssuerSelector::build(&cfg).is_err());
    }

    #[test]
    fn normalize_host_strips_port_case_and_trailing_dot() {
        let cases = [
            ("Portal.Example", "portal.example"),
            ("portal.example:8443", "portal.example"),
            ("portal.example.", "portal.example"),
            ("  portal.example  ", "portal.example"),
            ("[::1]:8443", "::1"),
            ("[2001:db8::1]", "2001:db8::1"),
            // Not a numeric port: leave the value alone (it just won't match).
            ("portal.example:x", "portal.example:x"),
        ];
        for (raw, expected) in cases {
            assert_eq!(normalize_host(raw), expected, "for {raw:?}");
        }
    }
}
