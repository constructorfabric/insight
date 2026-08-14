//! Gear configuration.
//!
//! Loaded via `GearCtx::config::<GearConfig>()` from the
//! `gears.git-cli-proxy.config` YAML section. Env overrides are
//! `APP__gears__git_cli_proxy__config__<field>`.
//!
//! Every operationally load-bearing field is **required**: the committed YAML
//! ships empty/zero placeholders and [`GearConfig::validate`] fails the boot
//! when a value is missing — no silent defaults for budgets, paths, or the
//! auth token (deployment supplies them via Helm).

use serde::Deserialize;

/// Configuration consumed by the git-cli-proxy gear. Deserialized from
/// `gears.git-cli-proxy.config`.
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct GearConfig {
    /// Address the HTTP host binds, `host:port`.
    pub bind_addr: String,
    /// Root directory of the repository cache (a PVC mount in K8s).
    pub data_dir: String,
    /// Total disk budget for the cache, bytes. Must sit 10–15% below the
    /// volume size (headroom for transient packs / git temp files).
    pub disk_budget_bytes: u64,
    /// Per-repository size cap, bytes. A clone/fetch that would exceed it is
    /// aborted and surfaced as a permanent (non-retryable) error.
    pub max_repo_bytes: u64,
    /// Default freshness window, seconds — a repo fetched more recently than
    /// this is served without contacting origin. Per-request override:
    /// `X-Max-Staleness` header.
    pub default_max_staleness_seconds: u64,
    /// Concurrency cap for heavy git operations (clone / fetch / repack).
    pub heavy_ops_concurrency: usize,
    /// Static bearer token protecting every `/v1` route (service-to-service
    /// auth; the service is cluster-internal and never behind the platform
    /// gateway). Supplied per-deployment, never committed.
    pub proxy_token: String,
    /// PEM bundle for origins whose TLS chain is not in the system store —
    /// a self-hosted vendor behind a private CA. Empty means "system store
    /// only", which is correct for the public clouds, so this is the one
    /// optional field.
    pub ca_cert_path: String,
    /// Accept `file://` origins. Test-harness escape hatch only: the hermetic
    /// suite clones from local fixture repositories. No deployment sets it —
    /// the chart hard-codes `false`.
    pub allow_file_repos: bool,
    /// Exceptions to the built-in refusal of loopback, link-local, private and
    /// cluster-internal names. Additive only: a public vendor needs no entry,
    /// so tenants add sources without an operator redeploying.
    #[serde(default, deserialize_with = "empty_when_null")]
    pub allowed_repo_hosts: Vec<String>,
}

/// Manual `Debug` that never prints the token — the config is logged on boot
/// failures and must stay secret-free.
/// `key:` with nothing after it is YAML null, and it is how a human writes an
/// empty list. Refusing to start on it is a worse answer than reading it as
/// one.
fn empty_when_null<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<Vec<String>>::deserialize(deserializer)?.unwrap_or_default())
}

impl std::fmt::Debug for GearConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GearConfig")
            .field("bind_addr", &self.bind_addr)
            .field("data_dir", &self.data_dir)
            .field("disk_budget_bytes", &self.disk_budget_bytes)
            .field("max_repo_bytes", &self.max_repo_bytes)
            .field(
                "default_max_staleness_seconds",
                &self.default_max_staleness_seconds,
            )
            .field("heavy_ops_concurrency", &self.heavy_ops_concurrency)
            .field("ca_cert_path", &self.ca_cert_path)
            .field("allow_file_repos", &self.allow_file_repos)
            .field("allowed_repo_hosts", &self.allowed_repo_hosts)
            .field("proxy_token", &"<redacted>")
            .finish()
    }
}

impl GearConfig {
    /// Fail-fast validation of the required fields. Called from gear init so a
    /// misconfigured deployment dies loudly at boot, not on the first request.
    ///
    /// # Errors
    ///
    /// Returns a single aggregated error naming every missing/invalid field
    /// (so one boot failure surfaces the whole gap, not the first field).
    pub fn validate(&self) -> anyhow::Result<()> {
        let mut missing: Vec<&str> = Vec::new();
        if self.bind_addr.is_empty() {
            missing.push("bind_addr (host:port the HTTP server binds)");
        }
        if self.data_dir.is_empty() {
            missing.push("data_dir (path to the repo cache volume)");
        }
        if self.disk_budget_bytes == 0 {
            missing.push("disk_budget_bytes (total cache budget, > 0)");
        }
        if self.max_repo_bytes == 0 {
            missing.push("max_repo_bytes (per-repo cap, > 0)");
        }
        if self.default_max_staleness_seconds == 0 {
            missing.push("default_max_staleness_seconds (freshness window, > 0)");
        }
        if self.heavy_ops_concurrency == 0 {
            missing.push("heavy_ops_concurrency (clone/fetch/repack cap, > 0)");
        }
        if self.proxy_token.is_empty() {
            missing.push("proxy_token (service-to-service bearer token)");
        }
        let watermark = crate::engine::disk::Budget {
            total_bytes: self.disk_budget_bytes,
        }
        .high_watermark();
        if self.max_repo_bytes > 0 && self.disk_budget_bytes > 0 && self.max_repo_bytes > watermark
        {
            missing.push(
                "max_repo_bytes must fit under the reclaim high watermark (85% of \
                 disk_budget_bytes) — admission reserves the per-repo cap before a clone, so a \
                 larger cap is refused even on an empty cache and every request 429s forever",
            );
        }
        anyhow::ensure!(
            missing.is_empty(),
            "invalid `gears.git-cli-proxy.config` (env: APP__gears__git_cli_proxy__config__*): {}",
            missing.join("; ")
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> GearConfig {
        GearConfig {
            bind_addr: "127.0.0.1:0".to_owned(),
            data_dir: "/data".to_owned(),
            disk_budget_bytes: 10_000_000_000,
            max_repo_bytes: 2_000_000_000,
            default_max_staleness_seconds: 300,
            heavy_ops_concurrency: 4,
            proxy_token: "secret".to_owned(),
            ca_cert_path: String::new(),
            allow_file_repos: false,
            allowed_repo_hosts: Vec::new(),
        }
    }

    #[test]
    fn valid_config_passes() {
        assert!(
            valid().validate().is_ok(),
            "fully populated config must validate"
        );
    }

    #[test]
    fn each_missing_field_fails_and_is_named() {
        let cases: Vec<(&str, GearConfig)> = vec![
            (
                "bind_addr",
                GearConfig {
                    bind_addr: String::new(),
                    ..valid()
                },
            ),
            (
                "data_dir",
                GearConfig {
                    data_dir: String::new(),
                    ..valid()
                },
            ),
            (
                "disk_budget_bytes",
                GearConfig {
                    disk_budget_bytes: 0,
                    ..valid()
                },
            ),
            (
                "max_repo_bytes",
                GearConfig {
                    max_repo_bytes: 0,
                    ..valid()
                },
            ),
            (
                "default_max_staleness_seconds",
                GearConfig {
                    default_max_staleness_seconds: 0,
                    ..valid()
                },
            ),
            (
                "heavy_ops_concurrency",
                GearConfig {
                    heavy_ops_concurrency: 0,
                    ..valid()
                },
            ),
            (
                "proxy_token",
                GearConfig {
                    proxy_token: String::new(),
                    ..valid()
                },
            ),
        ];
        for (field, cfg) in cases {
            let err = match cfg.validate() {
                Ok(()) => panic!("config with empty {field} must fail validation"),
                Err(e) => e.to_string(),
            };
            assert!(err.contains(field), "error must name `{field}`, got: {err}");
        }
    }

    #[test]
    fn default_config_fails_with_all_fields_listed() {
        let err = match GearConfig::default().validate() {
            Ok(()) => panic!("default (placeholder) config must fail validation"),
            Err(e) => e.to_string(),
        };
        for field in [
            "bind_addr",
            "data_dir",
            "disk_budget_bytes",
            "max_repo_bytes",
            "default_max_staleness_seconds",
            "heavy_ops_concurrency",
            "proxy_token",
        ] {
            assert!(
                err.contains(field),
                "aggregated error must name `{field}`, got: {err}"
            );
        }
    }

    #[test]
    fn repo_cap_above_the_reclaim_watermark_fails() {
        // Admission reserves max_repo_bytes before a clone, so a cap between
        // the high watermark and the budget boots fine and then refuses every
        // request forever — used=0, nothing to reclaim, 429 in a loop.
        let cases = vec![
            ("cap over the whole budget", 20_000_000_000_u64),
            (
                "cap inside the budget but over its 85% watermark",
                9_000_000_000,
            ),
        ];
        for (name, cap) in cases {
            let cfg = GearConfig {
                max_repo_bytes: cap,
                disk_budget_bytes: 10_000_000_000,
                ..valid()
            };
            let err = match cfg.validate() {
                Ok(()) => panic!("{name} must fail validation"),
                Err(e) => e.to_string(),
            };
            assert!(err.contains("high watermark"), "{name}: got: {err}");
        }

        let at_watermark = GearConfig {
            max_repo_bytes: 8_500_000_000,
            disk_budget_bytes: 10_000_000_000,
            ..valid()
        };
        assert!(
            at_watermark.validate().is_ok(),
            "a cap exactly at the watermark must be admissible"
        );
    }

    #[test]
    fn an_empty_allowlist_may_be_written_as_a_bare_key() {
        // `allowed_repo_hosts:` with nothing after it is YAML null. The chart
        // rendered exactly that for the default empty list and the service
        // refused to boot on it — no unit test saw it, because nothing parsed
        // the chart's own output.
        let json = r#"{
            "data_dir": "/data",
            "disk_budget_bytes": 10,
            "max_repo_bytes": 10,
            "default_max_staleness_seconds": 1,
            "heavy_ops_concurrency": 1,
            "proxy_token": "t",
            "ca_cert_path": "",
            "allow_file_repos": false,
            "allowed_repo_hosts": null
        }"#;
        match serde_json::from_str::<GearConfig>(json) {
            Ok(config) => assert!(config.allowed_repo_hosts.is_empty()),
            Err(e) => panic!("an explicit null must read as an empty list: {e}"),
        }
    }

    #[test]
    fn debug_never_prints_the_token() {
        let rendered = format!("{:?}", valid());
        assert!(
            !rendered.contains("secret"),
            "Debug leaked the token: {rendered}"
        );
        assert!(
            rendered.contains("<redacted>"),
            "Debug must mark the token redacted"
        );
    }
}
