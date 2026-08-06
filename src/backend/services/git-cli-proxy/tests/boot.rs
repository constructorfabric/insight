//! End-to-end tests against the built binary (routegen `tests/cli.rs`
//! precedent — llvm-cov instruments spawned binaries, so `main.rs` and the
//! gear wiring count). Proves what unit tests cannot: the host boots with the
//! minimal system-gear set, `/healthz` stays public, `/v1` is bearer-guarded,
//! an invalid config refuses to start, and a real repository is cloned and
//! served over HTTP.

use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

type R = Result<(), Box<dyn std::error::Error>>;

const TOKEN: &str = "boot-t0ken";

struct Server {
    child: Child,
    port: u16,
    dir: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn free_port() -> Result<u16, Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

fn test_dir(tag: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!(
        "git-cli-proxy-boot-{tag}-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(dir.join("data"))?;
    Ok(dir)
}

fn write_config(dir: &Path, port: u16, token: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
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
      listen_addr: "uds://{home}-grpc.sock"
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
        data = dir.join("data").display(),
    );
    let path = dir.join("insight.yaml");
    std::fs::write(&path, config)?;
    Ok(path)
}

fn spawn_server(tag: &str, token: &str) -> Result<Server, Box<dyn std::error::Error>> {
    let port = free_port()?;
    let dir = test_dir(tag)?;
    let config = write_config(&dir, port, token)?;

    let child = Command::new(env!("CARGO_BIN_EXE_git-cli-proxy"))
        .arg("--config")
        .arg(&config)
        .arg("run")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(Server { child, port, dir })
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

/// A fixture origin with partial-clone enabled, as a real vendor offers.
fn fixture_origin(root: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let origin = root.join("origin");
    std::fs::create_dir_all(&origin)?;
    // Explicit, distinct commit dates: the walk order is
    // (committed_date, sha), so same-second commits would order by sha and
    // make the assertions depend on hash values.
    let script = "git init -q -b main . && \
         git config uploadpack.allowFilter true && \
         git config uploadpack.allowAnySHA1InWant true && \
         echo one > a.txt && git add a.txt && \
         GIT_AUTHOR_DATE='2026-08-01T10:00:00+0000' GIT_COMMITTER_DATE='2026-08-01T10:00:00+0000' \
           git commit -qm 'first commit' && \
         echo two >> a.txt && echo new > b.txt && git add . && \
         GIT_AUTHOR_DATE='2026-08-02T11:00:00+0000' GIT_COMMITTER_DATE='2026-08-02T11:00:00+0000' \
           git commit -qm 'second commit'";
    let output = Command::new("sh")
        .arg("-ec")
        .arg(script)
        .current_dir(&origin)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "fixture setup failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(format!("file://{}", origin.display()))
}

/// Poll an endpoint until the cold clone finishes (the connector's 429 loop).
async fn get_json(
    port: u16,
    path: &str,
    repo: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}{path}");
    let deadline = Instant::now() + Duration::from_mins(1);
    loop {
        let response = client
            .get(&url)
            .query(&[("repo", repo)])
            .bearer_auth(TOKEN)
            .header("x-tenant-id", "tenant-1")
            .header("x-source-id", "source-1")
            .header("x-git-username", "u")
            .header("x-git-token", "p")
            .send()
            .await?;
        if response.status().as_u16() == 429 {
            if Instant::now() > deadline {
                return Err("repository never became ready".into());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
            continue;
        }
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(format!("{path} failed with {status}: {body}").into());
        }
        return Ok(serde_json::from_str(&body)?);
    }
}

#[tokio::test]
async fn boots_and_enforces_bearer_auth() -> R {
    let server = spawn_server("auth", TOKEN)?;
    wait_healthy(server.port).await?;

    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{}", server.port);

    let health = client.get(format!("{base}/healthz")).send().await?;
    assert_eq!(
        health.status(),
        200,
        "host-provided /healthz must be public"
    );

    let unauthenticated = client
        .get(format!("{base}/v1/branches?repo=x"))
        .send()
        .await?;
    assert_eq!(unauthenticated.status(), 401, "no token must be rejected");

    let wrong = client
        .get(format!("{base}/v1/branches?repo=x"))
        .bearer_auth("wrong")
        .send()
        .await?;
    assert_eq!(wrong.status(), 401, "wrong token must be rejected");

    let no_identity = client
        .get(format!("{base}/v1/branches?repo=x"))
        .bearer_auth(TOKEN)
        .send()
        .await?;
    assert_eq!(
        no_identity.status(),
        400,
        "the proxy token alone is not an identity"
    );
    Ok(())
}

#[tokio::test]
async fn serves_commits_file_changes_and_branches_from_a_clone() -> R {
    let server = spawn_server("serve", TOKEN)?;
    wait_healthy(server.port).await?;
    let repo = fixture_origin(&server.dir)?;

    let commits = get_json(server.port, "/v1/commits", &repo).await?;
    let items = commits["items"]
        .as_array()
        .ok_or("commits response has no items array")?;
    assert_eq!(items.len(), 2, "both fixture commits must be served");

    let first = &items[0];
    assert_eq!(first["message"], "first commit");
    assert_eq!(first["author_email"], "test@example.com");
    assert_eq!(first["is_merge"], false);
    assert_eq!(
        first["branch_names"]
            .as_array()
            .map(|names| names.iter().any(|n| n == "main")),
        Some(true),
        "branch membership must be reported"
    );
    assert!(
        first["patch_id"].is_string(),
        "patch_id must be computed: {first}"
    );
    assert!(
        items[0]["committed_date"].as_str() <= items[1]["committed_date"].as_str(),
        "commits must be ordered ascending by committed_date"
    );
    assert_eq!(
        items[1]["changed_files"], 2,
        "the second commit touches two files"
    );

    let changes = get_json(server.port, "/v1/file-changes", &repo).await?;
    let rows = changes["items"]
        .as_array()
        .ok_or("file-changes response has no items array")?;
    assert_eq!(rows.len(), 3, "1 file in c1 + 2 files in c2");
    let with_patch = rows
        .iter()
        .find(|row| row["filename"] == "b.txt")
        .ok_or("b.txt row missing")?;
    assert_eq!(with_patch["status"], "added");
    assert_eq!(with_patch["additions"], 1);
    assert_eq!(with_patch["patch_truncated"], false);
    assert!(
        with_patch["patch"]
            .as_str()
            .is_some_and(|patch| patch.contains("+new")),
        "patch text must be included by default: {with_patch}"
    );

    let branches = get_json(server.port, "/v1/branches", &repo).await?;
    let branch_rows = branches["items"]
        .as_array()
        .ok_or("branches response has no items array")?;
    assert_eq!(branch_rows.len(), 1);
    assert_eq!(branch_rows[0]["name"], "main");
    assert_eq!(
        branch_rows[0]["is_default"], true,
        "the mirrored HEAD marks the default branch"
    );
    assert!(branch_rows[0]["head_sha"].is_string());
    Ok(())
}

#[tokio::test]
async fn paginates_commits_with_a_snapshot_bound_cursor() -> R {
    let server = spawn_server("pages", TOKEN)?;
    wait_healthy(server.port).await?;
    let repo = fixture_origin(&server.dir)?;

    // Warm the cache first so the paging requests do not race the clone.
    get_json(server.port, "/v1/commits", &repo).await?;

    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{}", server.port);
    let first_page: serde_json::Value = client
        .get(format!("{base}/v1/commits"))
        .query(&[("repo", repo.as_str()), ("page_size", "1")])
        .bearer_auth(TOKEN)
        .header("x-tenant-id", "tenant-1")
        .header("x-source-id", "source-1")
        .header("x-git-username", "u")
        .header("x-git-token", "p")
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(
        first_page["items"].as_array().map(Vec::len),
        Some(1),
        "page_size must be honored"
    );
    let token = first_page["next_page_token"]
        .as_str()
        .ok_or("a truncated page must carry a cursor")?;

    let second_page: serde_json::Value = client
        .get(format!("{base}/v1/commits"))
        .query(&[
            ("repo", repo.as_str()),
            ("page_size", "1"),
            ("page_token", token),
        ])
        .bearer_auth(TOKEN)
        .header("x-tenant-id", "tenant-1")
        .header("x-source-id", "source-1")
        .header("x-git-username", "u")
        .header("x-git-token", "p")
        .send()
        .await?
        .json()
        .await?;

    let second_items = second_page["items"]
        .as_array()
        .ok_or("second page has no items")?;
    assert_eq!(second_items.len(), 1);
    assert_ne!(
        second_items[0]["sha"], first_page["items"][0]["sha"],
        "a continuation must not repeat the cursor row"
    );
    assert!(
        second_page["next_page_token"].is_null(),
        "the walk ends after the last commit"
    );
    Ok(())
}

#[tokio::test]
async fn sha_filter_selects_one_commit_across_both_endpoints() -> R {
    let server = spawn_server("sha", TOKEN)?;
    wait_healthy(server.port).await?;
    let repo = fixture_origin(&server.dir)?;

    let commits = get_json(server.port, "/v1/commits", &repo).await?;
    let items = commits["items"].as_array().ok_or("no items")?;
    let target = items[1]["sha"].as_str().ok_or("no sha")?.to_owned();
    let prefix = &target[..8];

    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{}", server.port);
    let ask = |path: &str, sha: String| {
        let url = format!("{base}{path}");
        let repo = repo.clone();
        let client = client.clone();
        async move {
            client
                .get(url)
                .query(&[("repo", repo.as_str()), ("sha", sha.as_str())])
                .bearer_auth(TOKEN)
                .header("x-tenant-id", "tenant-1")
                .header("x-source-id", "source-1")
                .header("x-git-username", "u")
                .header("x-git-token", "p")
                .send()
                .await?
                .json::<serde_json::Value>()
                .await
        }
    };

    let by_prefix = ask("/v1/commits", prefix.to_owned()).await?;
    let selected = by_prefix["items"].as_array().ok_or("no items")?;
    assert_eq!(selected.len(), 1, "an 8-char prefix selects one commit");
    assert_eq!(selected[0]["sha"], target.as_str());

    let changes = ask("/v1/file-changes", target.clone()).await?;
    let rows = changes["items"].as_array().ok_or("no items")?;
    assert!(!rows.is_empty(), "the selected commit has file rows");
    assert!(
        rows.iter().all(|row| row["sha"] == target.as_str()),
        "no other commit may leak into a filtered response"
    );

    let rejected = client
        .get(format!("{base}/v1/commits"))
        .query(&[("repo", repo.as_str()), ("sha", "nothex")])
        .bearer_auth(TOKEN)
        .header("x-tenant-id", "tenant-1")
        .header("x-source-id", "source-1")
        .header("x-git-username", "u")
        .header("x-git-token", "p")
        .send()
        .await?;
    assert_eq!(rejected.status(), 400, "a malformed sha is a bad request");
    Ok(())
}

#[tokio::test]
async fn refuses_to_boot_on_empty_required_config() -> R {
    let port = free_port()?;
    let dir = test_dir("invalid")?;
    let config = write_config(&dir, port, "")?;

    let mut child = Command::new(env!("CARGO_BIN_EXE_git-cli-proxy"))
        .arg("--config")
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
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
