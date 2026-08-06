//! Boot tests: drive the built binary end to end (routegen `tests/cli.rs`
//! precedent — llvm-cov instruments spawned binaries, so `main.rs` and
//! `gear.rs` count). Proves the host wiring that unit tests cannot: the
//! minimal system-gear set boots, `/healthz` is served by the host, `/v1` is
//! guarded by the bearer middleware, and an invalid config refuses to start.

use std::io::Read;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

type R = Result<(), Box<dyn std::error::Error>>;

struct Server {
    child: Child,
    port: u16,
    _dir: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> Result<u16, Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

fn test_dir(tag: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("git-cli-proxy-boot-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("data"))?;
    Ok(dir)
}

fn write_config(
    dir: &std::path::Path,
    port: u16,
    token: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let data_dir = dir.join("data");
    let config = format!(
        r#"server:
  home_dir: "{home}"
logging:
  default:
    console_level: info
gears:
  api-gateway:
    config:
      bind_addr: "127.0.0.1:{port}"
      enable_docs: false
      cors_enabled: false
      auth_disabled: true
  gear-orchestrator:
    config: {{}}
  authn-resolver:
    config:
      vendor: "hyperspot"
  grpc-hub:
    config:
      listen_addr: "uds:///tmp/git-cli-proxy-boot-{port}.sock"
  git-cli-proxy:
    config:
      data_dir: "{data}"
      disk_budget_bytes: 1000000000
      max_repo_bytes: 500000000
      default_max_staleness_seconds: 300
      heavy_ops_concurrency: 2
      proxy_token: "{token}"
"#,
        home = dir.join("home").display(),
        data = data_dir.display(),
    );
    let path = dir.join("insight.yaml");
    std::fs::write(&path, config)?;
    Ok(path)
}

fn spawn_server(token: &str) -> Result<Server, Box<dyn std::error::Error>> {
    let port = free_port()?;
    let dir = test_dir("run")?;
    let config = write_config(&dir, port, token)?;

    let child = Command::new(env!("CARGO_BIN_EXE_git-cli-proxy"))
        .args(["--config"])
        .arg(&config)
        .arg("run")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(Server {
        child,
        port,
        _dir: dir,
    })
}

async fn wait_healthy(port: u16) -> R {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/healthz");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(response) = client.get(&url).send().await
            && response.status().is_success()
        {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err("server did not become healthy within 30s".into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test]
async fn boots_and_enforces_bearer_auth() -> R {
    let mut server = spawn_server("boot-t0ken")?;
    wait_healthy(server.port).await?;

    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{}", server.port);

    let health = client.get(format!("{base}/healthz")).send().await?;
    assert_eq!(
        health.status(),
        200,
        "host-provided /healthz must be public"
    );

    let unauthenticated = client.get(format!("{base}/v1/ping")).send().await?;
    assert_eq!(unauthenticated.status(), 401, "no token must be rejected");

    let wrong = client
        .get(format!("{base}/v1/ping"))
        .bearer_auth("wrong")
        .send()
        .await?;
    assert_eq!(wrong.status(), 401, "wrong token must be rejected");

    let authenticated = client
        .get(format!("{base}/v1/ping"))
        .bearer_auth("boot-t0ken")
        .send()
        .await?;
    assert_eq!(authenticated.status(), 200, "configured token must pass");

    server.child.kill()?;
    Ok(())
}

#[tokio::test]
async fn refuses_to_boot_on_empty_required_config() -> R {
    let port = free_port()?;
    let dir = test_dir("invalid")?;
    // proxy_token intentionally empty — validate() must fail the boot.
    let config = write_config(&dir, port, "")?;

    let mut child = Command::new(env!("CARGO_BIN_EXE_git-cli-proxy"))
        .args(["--config"])
        .arg(&config)
        .arg("run")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;

    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            return Err("server must exit on invalid config, still running after 30s".into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert!(!status.success(), "boot with empty proxy_token must fail");

    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    assert!(
        stderr.contains("proxy_token"),
        "boot failure must name the missing field, got: {stderr}"
    );
    Ok(())
}
