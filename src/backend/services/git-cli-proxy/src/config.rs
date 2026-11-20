use serde::Deserialize;

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct GearConfig {
    pub bind_addr: String,
    pub data_dir: String,
    pub disk_budget_bytes: u64,
    pub max_repo_bytes: u64,
    pub default_max_staleness_seconds: u64,
    pub heavy_ops_concurrency: usize,
    pub serve_concurrency: usize,
    pub proxy_token: String,
    pub ca_cert_path: String,
    pub allow_file_repos: bool,
    #[serde(default, deserialize_with = "empty_when_null")]
    pub allowed_repo_hosts: Vec<String>,
}

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
            .field("serve_concurrency", &self.serve_concurrency)
            .field("ca_cert_path", &self.ca_cert_path)
            .field("allow_file_repos", &self.allow_file_repos)
            .field("allowed_repo_hosts", &self.allowed_repo_hosts)
            .field("proxy_token", &"<redacted>")
            .finish()
    }
}

impl GearConfig {
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
        if self.serve_concurrency == 0 {
            missing.push("serve_concurrency (page-serve cap, > 0)");
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
            serve_concurrency: 4,
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
                "serve_concurrency",
                GearConfig {
                    serve_concurrency: 0,
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
            "serve_concurrency",
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
