//! Semantic validation of `routes.yaml`, enforced before nginx ever sees the
//! generated config (gateway DESIGN section 3.8). Every violation fails the run
//! -- invalid YAML never reaches a pod.

use std::collections::HashSet;
use std::fmt;

use url::Url;

use crate::schema::{RouteConfig, SUPPORTED_VERSION};

/// All validation violations found in one pass, reported together so an operator
/// fixes a `routes.yaml` in one edit rather than one error at a time.
#[derive(Debug)]
pub struct ValidationErrors(pub Vec<String>);

impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "- {e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationErrors {}

/// Headers the gateway always controls; an operator must never strip them via
/// `strip_request_headers` (they are stripped or gateway-authored regardless,
/// and listing them signals a misunderstanding worth failing on). Matched
/// case-insensitively; `x-forwarded-*` is a prefix match. `X-Tenant-ID` is
/// deliberately NOT reserved -- it is the tenant selector the downstream
/// middleware validates against the signed `tenants[]`.
const RESERVED_HEADERS: &[&str] = &["authorization", "x-correlation-id", "cookie"];
const RESERVED_PREFIX: &str = "x-forwarded-";

/// Validate a parsed route table.
///
/// # Errors
/// Returns every violation found. See gateway DESIGN section 3.8 for the rules.
pub fn validate(config: &RouteConfig) -> Result<(), ValidationErrors> {
    let mut errors = Vec::new();

    if config.version != SUPPORTED_VERSION {
        errors.push(format!(
            "unsupported schema version {}: this configurator supports version {SUPPORTED_VERSION}",
            config.version
        ));
    }

    // Reserved headers must not appear in the operator strip list, and each entry
    // must be a syntactically valid HTTP header name.
    for header in &config.defaults.strip_request_headers {
        if !is_valid_header_name(header) {
            errors.push(format!(
                "defaults.strip_request_headers: '{header}' is not a valid HTTP header name"
            ));
        } else if is_reserved_header(header) {
            errors.push(format!(
                "defaults.strip_request_headers: '{header}' is gateway-reserved and stripped \
                 unconditionally -- remove it from this list"
            ));
        }
    }

    let mut seen_prefixes: HashSet<&str> = HashSet::new();
    for route in &config.routes {
        let p = route.prefix.as_str();

        if !seen_prefixes.insert(p) {
            errors.push(format!("duplicate route prefix '{p}'"));
        }

        if !p.starts_with("/api/") {
            errors.push(format!(
                "route prefix '{p}' must start with '/api/' (operator routes live only under /api/)"
            ));
        }

        match parse_upstream(&route.upstream) {
            Ok(()) => {}
            Err(reason) => errors.push(format!(
                "route '{p}': invalid upstream '{}': {reason}",
                route.upstream
            )),
        }

        let resolved = route.resolve(&config.defaults);
        if resolved.timeout_ms == 0 && !resolved.websocket {
            errors.push(format!(
                "route '{p}': timeout_ms 0 is only allowed with websocket: true"
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ValidationErrors(errors))
    }
}

/// An upstream must be an absolute `http`/`https` URL carrying an explicit host
/// and port (the emitter turns it into an nginx `upstream { server host:port; }`).
fn parse_upstream(raw: &str) -> Result<(), String> {
    let url = Url::parse(raw).map_err(|e| e.to_string())?;
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(format!("scheme '{other}' is not http/https")),
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err("missing host".to_owned());
    }
    // http/https always have a known default port, so this only fails for a
    // genuinely port-less, default-less scheme -- which the scheme check above
    // already rejects.
    if url.port_or_known_default().is_none() {
        return Err("missing port".to_owned());
    }
    if url.path() != "/" && !url.path().is_empty() {
        return Err(format!(
            "must not carry a path ('{}'); strip_prefix controls path rewriting",
            url.path()
        ));
    }
    Ok(())
}

fn is_reserved_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with(RESERVED_PREFIX) || RESERVED_HEADERS.contains(&lower.as_str())
}

/// The non-alphanumeric `tchar`s allowed in an RFC 7230 `token`.
const TCHAR_SYMBOLS: &[u8] = b"!#$%&'*+-.^_`|~";

/// RFC 7230 `token`: one or more `tchar`s (alphanumerics plus [`TCHAR_SYMBOLS`]).
fn is_valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || TCHAR_SYMBOLS.contains(&b))
}
