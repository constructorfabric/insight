//! nginx.conf emission (gateway DESIGN sections 3.9, 3.11; ADR-0001 Option A).
//!
//! The mechanism is the pure `access_by_lua` exchange, **not** the stock
//! `auth_request` directive: the two do not compose (an `auth_request` cannot be
//! skipped on a `lua_shared_dict` hit), so the miss-path subrequest is issued
//! from Lua. Every generated `/api/` location gets the full hygiene block by
//! construction -- there is no hand-written location to forget it in.

use std::fmt::Write as _;

use anyhow::Context as _;
use url::Url;

use crate::schema::{ResolvedRoute, RouteConfig};

/// Deployment settings that are not part of the route table itself (upstream
/// authorities, timeouts, trusted proxies). The CLI fills them from env at
/// container startup; defaults are the in-cluster values.
#[derive(Debug, Clone)]
pub struct Settings {
    pub listen: u16,
    pub authenticator_url: String,
    pub authz_path: String,
    pub front_url: String,
    pub jwt_cache_size: String,
    pub authz_connect_timeout_ms: u32,
    pub authz_read_timeout_ms: u32,
    pub worker_connections: u32,
    pub set_real_ip_from: Vec<String>,
    pub hsts: String,
    pub error_log_level: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            listen: 8080,
            authenticator_url: "http://authenticator.insight.svc.cluster.local:8083".to_owned(),
            authz_path: "/internal/authz".to_owned(),
            front_url: "http://insight-front.insight.svc.cluster.local:8080".to_owned(),
            jwt_cache_size: "64m".to_owned(),
            authz_connect_timeout_ms: 2000,
            authz_read_timeout_ms: 2000,
            worker_connections: 4096,
            set_real_ip_from: Vec::new(),
            hsts: "max-age=31536000; includeSubDomains; preload".to_owned(),
            error_log_level: "info".to_owned(),
        }
    }
}

/// A deduplicated upstream: one `upstream { server host:port; }` block, referenced
/// by one or more routes.
struct Upstream {
    ident: String,
    scheme: String,
    authority: String,
}

/// Parse `scheme://host:port` into `(scheme, host:port)`.
fn authority_of(raw: &str, what: &str) -> anyhow::Result<(String, String)> {
    let url = Url::parse(raw).with_context(|| format!("{what}: invalid URL '{raw}'"))?;
    let host = url
        .host_str()
        .with_context(|| format!("{what}: '{raw}' has no host"))?;
    let port = url
        .port_or_known_default()
        .with_context(|| format!("{what}: '{raw}' has no port"))?;
    Ok((url.scheme().to_owned(), format!("{host}:{port}")))
}

/// A stable, nginx-safe upstream identifier derived from the authority.
fn upstream_ident(authority: &str) -> String {
    let mut s = String::from("up_");
    for ch in authority.chars() {
        if ch.is_ascii_alphanumeric() {
            s.push(ch.to_ascii_lowercase());
        } else {
            s.push('_');
        }
    }
    s
}

/// Escape a route prefix for safe use inside an nginx `rewrite` regex.
fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(
            ch,
            '.' | '^' | '$' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// nginx time literal for a per-route timeout. `0` (websocket-only, validated)
/// becomes a long idle bound -- nginx has no "unlimited" read timeout.
fn timeout_literal(timeout_ms: u64) -> String {
    if timeout_ms == 0 {
        "3600s".to_owned()
    } else if timeout_ms.is_multiple_of(1000) {
        format!("{}s", timeout_ms / 1000)
    } else {
        format!("{timeout_ms}ms")
    }
}

/// Generate the complete nginx.conf for a validated route table.
///
/// # Errors
/// Fails if a settings URL (authenticator, front) or a route upstream cannot be
/// parsed into a host:port authority. Route upstreams are expected to have
/// passed [`crate::validate::validate`] first.
pub fn emit(config: &RouteConfig, settings: &Settings) -> anyhow::Result<String> {
    let routes = config.resolved_routes();

    // Deduplicate upstreams by authority, preserving first-seen order, and map
    // each route prefix to its upstream ident.
    let mut upstreams: Vec<Upstream> = Vec::new();
    let mut route_upstream: Vec<String> = Vec::with_capacity(routes.len());
    for route in &routes {
        let (scheme, authority) =
            authority_of(&route.upstream, &format!("route '{}'", route.prefix))?;
        let ident = upstream_ident(&authority);
        if !upstreams.iter().any(|u| u.ident == ident) {
            upstreams.push(Upstream {
                ident: ident.clone(),
                scheme,
                authority,
            });
        }
        route_upstream.push(ident);
    }

    let (_, auth_authority) = authority_of(&settings.authenticator_url, "authenticator url")?;
    // Validate the front URL early; the front location resolves it lazily
    // (variable proxy_pass) inside emit_server so a down/absent SPA doesn't stop
    // the gateway booting.
    let _ = authority_of(&settings.front_url, "front url")?;
    let authz_url = format!(
        "{}{}",
        settings.authenticator_url.trim_end_matches('/'),
        settings.authz_path
    );

    let mut c = String::new();

    // ── header ────────────────────────────────────────────────────────────
    c.push_str(
        "# Generated by routegen from routes.yaml at container startup -- DO NOT EDIT.\n\
         # Source of truth: routes.yaml (shipped as a ConfigMap / compose mount).\n\
         # A route change is a PR to routes.yaml; the container regenerates on restart.\n\
         # See docs/components/backend/gateway/DESIGN.md (DD-GW-02, section 3.13).\n\n",
    );

    writeln!(c, "worker_processes auto;")?;
    writeln!(c, "error_log /dev/stderr {};", settings.error_log_level)?;
    writeln!(c, "pid /tmp/nginx.pid;")?;
    c.push('\n');
    writeln!(c, "events {{")?;
    writeln!(c, "    worker_connections {};", settings.worker_connections)?;
    writeln!(c, "}}")?;
    c.push('\n');

    // ── http ──────────────────────────────────────────────────────────────
    writeln!(c, "http {{")?;
    writeln!(c, "    server_tokens off;")?;
    c.push('\n');

    emit_http_runtime(&mut c, settings, &authz_url)?;

    // Upstreams (keepalive-pooled).
    c.push_str("    # --- upstreams (keepalive-pooled) ---\n");
    writeln!(
        c,
        "    upstream authenticator {{ server {auth_authority}; keepalive 32; }}"
    )?;
    // NB: no static `upstream insight_front` — the SPA front is resolved lazily
    // in `location /` (variable proxy_pass + the http-block resolver) so the
    // gateway starts even when the frontend is absent (backend-only / API + auth
    // dev against a separately-run SPA). See emit_server.
    for u in &upstreams {
        writeln!(
            c,
            "    upstream {} {{ server {}; keepalive 32; }}",
            u.ident, u.authority
        )?;
    }
    c.push('\n');

    // Coarse flood guard (G8).
    c.push_str(
        "    # coarse per-IP flood guard for /auth/* (G8); precise limiting is in the authenticator\n",
    );
    c.push_str("    limit_req_zone $binary_remote_addr zone=auth_per_ip:10m rate=60r/m;\n\n");

    emit_server(
        &mut c,
        config,
        settings,
        &routes,
        &route_upstream,
        &upstreams,
    )?;

    writeln!(c, "}}")?;

    Ok(c)
}

/// Emit the http-block runtime: the Lua runtime + exchange config, the JSON
/// access log, and the client-IP trust chain.
fn emit_http_runtime(c: &mut String, settings: &Settings, authz_url: &str) -> anyhow::Result<()> {
    // OpenResty Lua runtime (ADR-0001 Option A).
    c.push_str("    # --- OpenResty Lua runtime (DESIGN 3.11; ADR-0001 Option A) ---\n");
    writeln!(c, "    lua_package_path \"/etc/nginx/lua/?.lua;;\";")?;
    writeln!(
        c,
        "    lua_shared_dict jwt_cache {};",
        settings.jwt_cache_size
    )?;
    // local=on reads nameservers from /etc/resolv.conf -- one config for
    // in-cluster (kube-dns) and compose (docker DNS), no per-cluster address.
    c.push_str("    resolver local=on ipv6=off;\n");
    c.push('\n');
    c.push_str("    init_by_lua_block {\n");
    c.push_str("        require(\"gateway\").init({\n");
    writeln!(c, "            authz_url = \"{authz_url}\",")?;
    writeln!(
        c,
        "            authz_connect_timeout_ms = {},",
        settings.authz_connect_timeout_ms
    )?;
    writeln!(
        c,
        "            authz_read_timeout_ms = {},",
        settings.authz_read_timeout_ms
    )?;
    c.push_str("        })\n");
    c.push_str("    }\n\n");

    // Access logging: JSON, never a cookie or JWT (DESIGN 3.14).
    c.push_str("    # --- access log: JSON, never a cookie or a JWT (DESIGN 3.14) ---\n");
    c.push_str("    log_format gateway_json escape=json\n");
    c.push_str("        '{'\n");
    c.push_str("            '\"time\":\"$time_iso8601\",'\n");
    c.push_str("            '\"remote_addr\":\"$remote_addr\",'\n");
    c.push_str("            '\"method\":\"$request_method\",'\n");
    c.push_str("            '\"uri\":\"$uri\",'\n");
    c.push_str("            '\"status\":$status,'\n");
    c.push_str("            '\"request_time\":$request_time,'\n");
    c.push_str("            '\"upstream_time\":\"$upstream_response_time\",'\n");
    c.push_str("            '\"upstream_addr\":\"$upstream_addr\",'\n");
    c.push_str("            '\"correlation_id\":\"$correlation_id\"'\n");
    c.push_str("        '}';\n");
    c.push_str("    access_log /dev/stdout gateway_json;\n\n");

    // Client-IP truth: trust X-Forwarded-For only from the explicitly configured
    // ingress hops (G8). Emitted only when trusted CIDRs are supplied -- there is
    // no baked default network, since the trusted range is deployment-specific.
    if !settings.set_real_ip_from.is_empty() {
        c.push_str("    # --- client-IP truth: trust only the configured ingress hops (G8) ---\n");
        c.push_str("    real_ip_header X-Forwarded-For;\n");
        c.push_str("    real_ip_recursive on;\n");
        for cidr in &settings.set_real_ip_from {
            writeln!(c, "    set_real_ip_from {cidr};")?;
        }
        c.push('\n');
    }

    c.push_str("    proxy_http_version 1.1;\n");
    // Writable temp paths under /tmp so the gateway runs as a non-root user with
    // a read-only root filesystem (the prefix dirs are not writable then).
    c.push_str("    # writable temp paths -> non-root + read-only rootfs\n");
    c.push_str("    client_body_temp_path /tmp/client_body;\n");
    c.push_str("    proxy_temp_path /tmp/proxy;\n");
    c.push_str("    fastcgi_temp_path /tmp/fastcgi;\n");
    c.push_str("    uwsgi_temp_path /tmp/uwsgi;\n");
    c.push_str("    scgi_temp_path /tmp/scgi;\n\n");
    Ok(())
}

/// Emit the `server { ... }` block: health, the fixed unauthenticated surface,
/// every generated `/api/` route, and the SPA fallthrough.
fn emit_server(
    c: &mut String,
    config: &RouteConfig,
    settings: &Settings,
    routes: &[ResolvedRoute],
    route_upstream: &[String],
    upstreams: &[Upstream],
) -> anyhow::Result<()> {
    writeln!(c, "    server {{")?;
    writeln!(c, "        listen {};", settings.listen)?;
    c.push_str("        server_name _;\n\n");
    c.push_str("        # correlation id is written per request by the Lua exchange\n");
    c.push_str("        set $correlation_id \"-\";\n\n");
    c.push_str("        # the gateway owns security headers regardless of ingress (G9/DD-GW-04)\n");
    writeln!(
        c,
        "        add_header Strict-Transport-Security \"{}\" always;",
        settings.hsts
    )?;
    c.push('\n');

    c.push_str("        # --- health: liveness + LOCAL readiness, never gated on a dependency (3.15) ---\n");
    c.push_str("        location = /healthz {\n");
    c.push_str("            access_log off;\n");
    c.push_str("            default_type text/plain;\n");
    c.push_str("            return 200 \"ok\\n\";\n");
    c.push_str("        }\n\n");

    // Auth API (plain proxy, no exchange -- it IS the auth). JWKS is deliberately
    // NOT fronted here: it is public, read-only, and consumed by downstream
    // services, which fetch it directly from the authenticator (the key issuer),
    // not through the edge.
    c.push_str("        # --- auth API: plain proxy, no exchange (it IS the auth) ---\n");
    c.push_str("        location /auth/ {\n");
    c.push_str("            limit_req zone=auth_per_ip burst=120 nodelay;\n");
    c.push_str("            proxy_pass http://authenticator;\n");
    c.push_str("            proxy_set_header Connection \"\";\n");
    c.push_str("            proxy_set_header Host $host;\n");
    c.push_str("            proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n");
    c.push_str("            proxy_set_header X-Forwarded-Proto $scheme;\n");
    c.push_str("        }\n\n");

    // Generated /api routes.
    c.push_str("        # --- generated /api routes: full auth + hygiene block per location ---\n");
    for (route, ident) in routes.iter().zip(route_upstream) {
        emit_api_location(c, route, ident, upstreams, config)?;
    }

    // Unmatched /api, /internal, SPA.
    c.push_str(
        "        # unmatched /api/* -> 404 (longest-prefix routing means the routes above win)\n",
    );
    c.push_str("        location /api/ {\n");
    c.push_str("            return 404;\n");
    c.push_str("        }\n\n");
    c.push_str("        # /internal/* never routes through the gateway (G4 layer 1)\n");
    c.push_str("        location /internal/ {\n");
    c.push_str("            return 404;\n");
    c.push_str("        }\n\n");
    c.push_str(
        "        # the SPA rides through the gateway: one origin, one __Host- cookie (DD-GW-04)\n",
    );
    // Resolve the SPA upstream LAZILY via a variable + the http-block resolver,
    // so the gateway boots even when the frontend is absent (backend-only / API
    // + auth dev against a separately-run SPA). `/` then 502s until a front is
    // up, but `/api/*` and `/auth/*` work. A static `upstream {}` would instead
    // fail nginx config load with "host not found in upstream".
    let (front_scheme, front_authority) = authority_of(&settings.front_url, "front url")?;
    c.push_str("        location / {\n");
    writeln!(c, "            set $insight_front \"{front_authority}\";")?;
    writeln!(c, "            proxy_pass {front_scheme}://$insight_front;")?;
    c.push_str("            proxy_set_header Connection \"\";\n");
    c.push_str("            proxy_set_header Host $host;\n");
    c.push_str("            proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n");
    c.push_str("            proxy_set_header X-Forwarded-Proto $scheme;\n");
    c.push_str("        }\n");

    writeln!(c, "    }}")?;

    Ok(())
}

/// Emit one `/api/` location with the complete hygiene block (DESIGN 3.9).
fn emit_api_location(
    c: &mut String,
    route: &ResolvedRoute,
    ident: &str,
    upstreams: &[Upstream],
    config: &RouteConfig,
) -> anyhow::Result<()> {
    let scheme = upstreams
        .iter()
        .find(|u| u.ident == *ident)
        .map_or("http", |u| u.scheme.as_str());

    writeln!(c, "        # route: {} -> {}", route.prefix, route.upstream)?;
    writeln!(c, "        location {} {{", route.prefix)?;
    // 1. auth exchange (Authorization inject, cookie strip, UUIDv7 -- all in Lua)
    c.push_str("            access_by_lua_block { require(\"gateway\").exchange() }\n");
    if route.strip_prefix {
        writeln!(
            c,
            "            rewrite ^{}/?(.*)$ /$1 break;",
            regex_escape(&route.prefix)
        )?;
    }
    writeln!(c, "            proxy_pass {scheme}://{ident};")?;
    c.push_str("            proxy_set_header Host $host;\n");
    // 5. gateway-authored forwarding headers (client-supplied are cleared in Lua)
    if route.websocket {
        c.push_str("            proxy_set_header Upgrade $http_upgrade;\n");
        c.push_str("            proxy_set_header Connection \"upgrade\";\n");
    } else {
        c.push_str("            proxy_set_header Connection \"\";\n");
    }
    c.push_str("            proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n");
    c.push_str("            proxy_set_header X-Forwarded-Proto $scheme;\n");
    // 6. operator strip_request_headers (reserved set is handled in Lua)
    for header in &config.defaults.strip_request_headers {
        writeln!(c, "            proxy_set_header {header} \"\";")?;
    }
    // 7. bounded connect (fail fast on a dead upstream -> 502/504, never hang) +
    //    per-route read/send timeouts + streaming
    c.push_str("            proxy_connect_timeout 5s;\n");
    let t = timeout_literal(route.timeout_ms);
    writeln!(c, "            proxy_read_timeout {t};")?;
    writeln!(c, "            proxy_send_timeout {t};")?;
    c.push_str("            proxy_buffering off;\n");
    c.push_str("        }\n\n");
    Ok(())
}
