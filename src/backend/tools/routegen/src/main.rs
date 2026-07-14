//! routegen CLI: compile `routes.yaml` into a complete `nginx.conf`.
//!
//! ```text
//! routegen --routes routes.yaml -o nginx.conf
//! routegen --routes routes.yaml --check nginx.conf   # CI up-to-date assertion
//! ```

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context as _;
use clap::Parser;
use routegen::Settings;

/// Compile a reviewable `routes.yaml` into the full gateway `nginx.conf`.
#[derive(Debug, Parser)]
#[command(name = "routegen", version, about)]
struct Cli {
    /// Path to the input `routes.yaml`.
    #[arg(long, default_value = "routes.yaml")]
    routes: PathBuf,

    /// Write the generated config here (default: stdout).
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Assert the file at this path already matches the generated config, then
    /// exit non-zero if it drifted. Used by CI to keep the committed config in
    /// sync with `routes.yaml`.
    #[arg(long, conflicts_with = "output")]
    check: Option<PathBuf>,

    /// Base URL of the authenticator (host:port used for the /auth upstream and
    /// the Lua exchange cosocket).
    #[arg(long, default_value_t = Settings::default().authenticator_url)]
    authenticator_url: String,

    /// Path on the authenticator that the Lua exchange calls.
    #[arg(long, default_value_t = Settings::default().authz_path)]
    authz_path: String,

    /// Base URL of the insight-front SPA server.
    #[arg(long, default_value_t = Settings::default().front_url)]
    front_url: String,

    /// Size of the `lua_shared_dict` exchange cache.
    #[arg(long, default_value_t = Settings::default().jwt_cache_size)]
    jwt_cache_size: String,

    /// Cosocket connect timeout to the authenticator, in ms.
    #[arg(long, default_value_t = Settings::default().authz_connect_timeout_ms)]
    authz_connect_timeout_ms: u32,

    /// Cosocket read timeout for the authenticator response, in ms.
    #[arg(long, default_value_t = Settings::default().authz_read_timeout_ms)]
    authz_read_timeout_ms: u32,

    /// Port the gateway listens on.
    #[arg(long, default_value_t = Settings::default().listen)]
    listen: u16,

    /// Trusted ingress-hop CIDR(s) for `set_real_ip_from` (repeatable).
    #[arg(long = "set-real-ip-from")]
    set_real_ip_from: Vec<String>,
}

impl Cli {
    fn settings(&self) -> Settings {
        let d = Settings::default();
        Settings {
            listen: self.listen,
            authenticator_url: self.authenticator_url.clone(),
            authz_path: self.authz_path.clone(),
            front_url: self.front_url.clone(),
            jwt_cache_size: self.jwt_cache_size.clone(),
            authz_connect_timeout_ms: self.authz_connect_timeout_ms,
            authz_read_timeout_ms: self.authz_read_timeout_ms,
            worker_connections: d.worker_connections,
            set_real_ip_from: if self.set_real_ip_from.is_empty() {
                d.set_real_ip_from
            } else {
                self.set_real_ip_from.clone()
            },
            hsts: d.hsts,
            error_log_level: d.error_log_level,
        }
    }
}

fn run(cli: &Cli) -> anyhow::Result<()> {
    let yaml = fs::read_to_string(&cli.routes)
        .with_context(|| format!("read routes file {}", cli.routes.display()))?;
    let generated = routegen::generate(&yaml, &cli.settings())?;

    if let Some(check_path) = &cli.check {
        let current = fs::read_to_string(check_path)
            .with_context(|| format!("read config to check {}", check_path.display()))?;
        if current != generated {
            anyhow::bail!(
                "{} is out of date with {} -- regenerate it with routegen and commit the result",
                check_path.display(),
                cli.routes.display()
            );
        }
        return Ok(());
    }

    match &cli.output {
        Some(path) => fs::write(path, generated)
            .with_context(|| format!("write generated config to {}", path.display()))?,
        None => print!("{generated}"),
    }
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("routegen: {e:#}");
            ExitCode::FAILURE
        }
    }
}
