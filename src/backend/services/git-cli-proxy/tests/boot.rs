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
    // nosemgrep: rust.lang.security.temp-dir.temp-dir -- test fixture; names carry pid/thread/counter and hold no secrets
    let dir = std::env::temp_dir().join(format!(
        "git-cli-proxy-boot-{tag}-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(dir.join("data"))?;
    Ok(dir)
}

fn write_config(
    dir: &Path,
    port: u16,
    token: &str,
    allow_file_repos: bool,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let config = format!(
        r#"server:
  home_dir: "{home}"
logging:
  default:
    console_level: info
gears:
  gear-orchestrator:
    config: {{}}
  grpc-hub:
    config:
      listen_addr: "uds://{home}-grpc.sock"
  git-cli-proxy:
    config:
      bind_addr: "127.0.0.1:{port}"
      data_dir: "{data}"
      disk_budget_bytes: 1000000000
      max_repo_bytes: 500000000
      default_max_staleness_seconds: 300
      heavy_ops_concurrency: 2
      proxy_token: "{token}"
      allow_file_repos: {allow_file_repos}
"#,
        home = dir.join("home").display(),
        data = dir.join("data").display(),
    );
    let path = dir.join("insight.yaml");
    std::fs::write(&path, config)?;
    Ok(path)
}

fn spawn_server(tag: &str, token: &str) -> Result<Server, Box<dyn std::error::Error>> {
    spawn_server_with(tag, token, true)
}

fn spawn_server_with(
    tag: &str,
    token: &str,
    allow_file_repos: bool,
) -> Result<Server, Box<dyn std::error::Error>> {
    let port = free_port()?;
    let dir = test_dir(tag)?;
    let config = write_config(&dir, port, token, allow_file_repos)?;

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

/// A default branch plus a side branch that was never merged into it.
fn branching_origin(root: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let origin = root.join("branching");
    std::fs::create_dir_all(&origin)?;
    let script = "git init -q -b main . && \
         git config uploadpack.allowFilter true && \
         git config uploadpack.allowAnySHA1InWant true && \
         echo a > a.txt && git add a.txt && \
         GIT_AUTHOR_DATE='2026-08-01T10:00:00+0000' GIT_COMMITTER_DATE='2026-08-01T10:00:00+0000' \
           git commit -qm 'on main' && \
         git checkout -q -b side && \
         echo b > b.txt && git add b.txt && \
         GIT_AUTHOR_DATE='2026-08-02T10:00:00+0000' GIT_COMMITTER_DATE='2026-08-02T10:00:00+0000' \
           git commit -qm 'on the side branch' && \
         git checkout -q main";
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

/// A history where one commit copies a file verbatim while modifying the
/// original — the shape `-C` detects and `-M` alone reports as an addition.
fn copying_origin(root: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let origin = root.join("copying");
    std::fs::create_dir_all(&origin)?;
    let script = "git init -q -b main . && \
         git config uploadpack.allowFilter true && \
         git config uploadpack.allowAnySHA1InWant true && \
         printf 'alpha\\nbeta\\ngamma\\ndelta\\nepsilon\\nzeta\\n' > a.txt && git add a.txt && \
         GIT_AUTHOR_DATE='2026-08-01T10:00:00+0000' GIT_COMMITTER_DATE='2026-08-01T10:00:00+0000' \
           git commit -qm 'base' && \
         cp a.txt b.txt && printf 'eta\\n' >> a.txt && git add -A && \
         GIT_AUTHOR_DATE='2026-08-02T10:00:00+0000' GIT_COMMITTER_DATE='2026-08-02T10:00:00+0000' \
           git commit -qm 'copy a to b'";
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

/// A superproject holding a submodule. The gitlink's object id is a commit in
/// the inner repository, which the outer origin has never heard of.
fn submodule_origin(root: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let inner = root.join("inner");
    let outer = root.join("outer");
    std::fs::create_dir_all(&inner)?;
    std::fs::create_dir_all(&outer)?;

    let setup = |dir: &Path, script: &str| -> Result<(), Box<dyn std::error::Error>> {
        let output = Command::new("sh")
            .arg("-ec")
            .arg(script)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .env("GIT_AUTHOR_DATE", "2026-08-01T10:00:00+0000")
            .env("GIT_COMMITTER_DATE", "2026-08-01T10:00:00+0000")
            .output()?;
        if output.status.success() {
            return Ok(());
        }
        Err(format!(
            "fixture setup failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into())
    };

    setup(
        &inner,
        "git init -q -b main . && echo i > i.txt && git add . && git commit -qm inner",
    )?;
    setup(
        &outer,
        &format!(
            "git init -q -b main . && \
             git config uploadpack.allowFilter true && \
             git config uploadpack.allowAnySHA1InWant true && \
             git -c protocol.file.allow=always submodule add -q '{}' sub && \
             echo o > o.txt && git add -A && git commit -qm 'add a submodule'",
            inner.display()
        ),
    )?;
    Ok(format!("file://{}", outer.display()))
}

/// An origin whose filenames exercise every spelling git C-quotes: a quote, a
/// backslash, a tab, a non-ASCII byte, a space, and a path holding the ` b/`
/// sequence that a `diff --git` header cannot resolve on its own.
fn awkward_paths_origin(root: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let origin = root.join("awkward");
    std::fs::create_dir_all(&origin)?;
    let script = "git init -q -b main . && \
         git config uploadpack.allowFilter true && \
         git config uploadpack.allowAnySHA1InWant true && \
         mkdir -p 'dir with space' 'has b' && \
         echo s > 'dir with space/a b.txt' && \
         echo q > 'quote\".txt' && \
         echo k > 'back\\slash.txt' && \
         echo u > 'unicode-ä.txt' && \
         echo n > 'has b/nested.txt' && \
         echo t > \"$(printf 'tab\\there.txt')\" && \
         git add -A && \
         GIT_AUTHOR_DATE='2026-08-01T10:00:00+0000' GIT_COMMITTER_DATE='2026-08-01T10:00:00+0000' \
           git commit -qm 'awkward names'";
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

/// A history whose committer dates run BACKWARDS along ancestry — routine after
/// a rebase, a cherry-pick, or a clock-skewed contributor. `git log --since` is
/// a traversal cutoff, so any walk narrowed to a cursor date prunes the older
/// parents and never reaches the commits behind them.
fn skewed_origin(root: &Path, name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let origin = root.join(name);
    std::fs::create_dir_all(&origin)?;
    let script = "git init -q -b main . && \
         git config uploadpack.allowFilter true && \
         git config uploadpack.allowAnySHA1InWant true && \
         echo a > a.txt && git add a.txt && \
         GIT_AUTHOR_DATE='2026-08-03T10:00:00+0000' GIT_COMMITTER_DATE='2026-08-03T10:00:00+0000' \
           git commit -qm 'newest ancestor' && \
         echo b > b.txt && git add b.txt && \
         GIT_AUTHOR_DATE='2026-08-01T10:00:00+0000' GIT_COMMITTER_DATE='2026-08-01T10:00:00+0000' \
           git commit -qm 'older descendant' && \
         echo c > c.txt && git add c.txt && \
         GIT_AUTHOR_DATE='2026-08-02T10:00:00+0000' GIT_COMMITTER_DATE='2026-08-02T10:00:00+0000' \
           git commit -qm 'middle descendant'";
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
async fn non_http_origins_are_refused_in_the_shipped_configuration() -> R {
    let server = spawn_server_with("scheme", TOKEN, false)?;
    wait_healthy(server.port).await?;
    let repo = fixture_origin(&server.dir)?;

    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{}", server.port);

    // The fixture origin is a real, reachable repository — only its transport
    // is refused, so a 400 here cannot be a false pass from an unreachable URL.
    for origin in [repo.as_str(), "ext::sh -c id", "/etc/passwd"] {
        let response = client
            .get(format!("{base}/v1/branches"))
            .query(&[("repo", origin)])
            .bearer_auth(TOKEN)
            .header("x-tenant-id", "t")
            .header("x-source-id", "s")
            .header("x-git-username", "u")
            .header("x-git-token", "p")
            .send()
            .await?;
        assert_eq!(response.status(), 400, "must refuse origin {origin:?}");
    }
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
        first["is_in_default_branch"], true,
        "default-branch membership must be reported"
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
async fn pagination_never_drops_commits_with_non_monotonic_dates() -> R {
    let server = spawn_server("skew", TOKEN)?;
    wait_healthy(server.port).await?;
    let repo = skewed_origin(&server.dir, "skewed")?;

    let unpaged = get_json(server.port, "/v1/commits", &repo).await?;
    let expected: Vec<String> = unpaged["items"]
        .as_array()
        .ok_or("no items")?
        .iter()
        .filter_map(|row| row["sha"].as_str().map(ToOwned::to_owned))
        .collect();
    assert_eq!(expected.len(), 3, "the fixture has three commits");

    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{}", server.port);
    let mut seen: Vec<String> = Vec::new();
    let mut token: Option<String> = None;

    loop {
        let mut params = vec![("repo", repo.clone()), ("page_size", "1".to_owned())];
        if let Some(cursor) = &token {
            params.push(("page_token", cursor.clone()));
        }
        let page: serde_json::Value = client
            .get(format!("{base}/v1/commits"))
            .query(&params)
            .bearer_auth(TOKEN)
            .header("x-tenant-id", "tenant-1")
            .header("x-source-id", "source-1")
            .header("x-git-username", "u")
            .header("x-git-token", "p")
            .send()
            .await?
            .json()
            .await?;

        for row in page["items"].as_array().ok_or("no items")? {
            seen.push(row["sha"].as_str().ok_or("no sha")?.to_owned());
        }
        match page["next_page_token"].as_str() {
            Some(next) => token = Some(next.to_owned()),
            None => break,
        }
    }

    assert_eq!(
        seen, expected,
        "paging must return exactly the unpaginated walk, in the same order"
    );
    Ok(())
}

#[tokio::test]
async fn a_commit_only_on_a_side_branch_is_not_in_the_default_branch() -> R {
    let server = spawn_server("sidebranch", TOKEN)?;
    wait_healthy(server.port).await?;
    let repo = branching_origin(&server.dir)?;

    let commits = get_json(server.port, "/v1/commits", &repo).await?;
    let items = commits["items"].as_array().ok_or("no items")?;

    let by_message = |message: &str| -> Option<bool> {
        items
            .iter()
            .find(|row| row["message"] == message)
            .and_then(|row| row["is_in_default_branch"].as_bool())
    };

    assert_eq!(
        by_message("on main"),
        Some(true),
        "a commit on the default branch must be marked"
    );
    assert_eq!(
        by_message("on the side branch"),
        Some(false),
        "a commit that never reached the default branch must not be"
    );
    Ok(())
}

#[tokio::test]
async fn since_returns_every_reachable_commit_at_or_after_it() -> R {
    let server = spawn_server("sincefilter", TOKEN)?;
    wait_healthy(server.port).await?;
    // Ancestry: 08-03 (root) <- 08-01 <- 08-02. `git log --since` stops
    // descending at the first commit older than the bound, so asking for
    // >= 08-02 would reach only the tip and prune the 08-03 root behind it.
    let repo = skewed_origin(&server.dir, "sincefilter")?;

    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{}", server.port);
    get_json(server.port, "/v1/commits", &repo).await?;

    let filtered: serde_json::Value = client
        .get(format!("{base}/v1/commits"))
        .query(&[("repo", repo.as_str()), ("since", "2026-08-02T00:00:00Z")])
        .bearer_auth(TOKEN)
        .header("x-tenant-id", "tenant-1")
        .header("x-source-id", "source-1")
        .header("x-git-username", "u")
        .header("x-git-token", "p")
        .send()
        .await?
        .json()
        .await?;

    let messages: Vec<&str> = filtered["items"]
        .as_array()
        .ok_or("no items")?
        .iter()
        .filter_map(|row| row["message"].as_str())
        .collect();

    assert!(
        messages.contains(&"newest ancestor"),
        "a qualifying commit behind an older parent must still be returned: {messages:?}"
    );
    assert!(
        messages.contains(&"middle descendant"),
        "the tip at the bound must be returned: {messages:?}"
    );
    assert!(
        !messages.contains(&"older descendant"),
        "a commit before the bound must be excluded: {messages:?}"
    );
    Ok(())
}

/// A branch that exists only to be deleted, plus a tag pinning its tip.
fn taggable_origin(root: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let origin = root.join("taggable");
    std::fs::create_dir_all(&origin)?;
    let script = "git init -q -b main . && \
         git config uploadpack.allowFilter true && \
         git config uploadpack.allowAnySHA1InWant true && \
         echo a > a.txt && git add a.txt && \
         GIT_AUTHOR_DATE='2026-08-01T10:00:00+0000' GIT_COMMITTER_DATE='2026-08-01T10:00:00+0000' \
           git commit -qm 'on main' && \
         git checkout -q -b doomed && \
         echo b > b.txt && git add b.txt && \
         GIT_AUTHOR_DATE='2026-08-02T10:00:00+0000' GIT_COMMITTER_DATE='2026-08-02T10:00:00+0000' \
           git commit -qm 'only on the doomed branch' && \
         git tag v1 && git checkout -q main";
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

fn run_in(dir: &Path, script: &str) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("sh")
        .arg("-ec")
        .arg(script)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
    }
    Ok(())
}

#[tokio::test]
async fn a_deleted_branch_stops_being_enumerated_even_when_a_tag_pins_it() -> R {
    let server = spawn_server("tagprune", TOKEN)?;
    wait_healthy(server.port).await?;
    let repo = taggable_origin(&server.dir)?;

    let before = get_json(server.port, "/v1/commits", &repo).await?;
    let count = |v: &serde_json::Value| {
        v["items"].as_array().map_or(0, |rows| {
            rows.iter()
                .filter(|r| r["message"] == "only on the doomed branch")
                .count()
        })
    };
    assert_eq!(
        count(&before),
        1,
        "the commit is reachable while the branch lives"
    );

    // Delete the branch at origin. The tag still pins its tip — and a mirror
    // refspec prunes refs/heads only, so `--all` would keep enumerating it.
    run_in(&server.dir.join("taggable"), "git branch -qD doomed")?;

    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{}", server.port);
    let after: serde_json::Value = client
        .get(format!("{base}/v1/commits"))
        .query(&[("repo", repo.as_str())])
        .bearer_auth(TOKEN)
        .header("x-tenant-id", "tenant-1")
        .header("x-source-id", "source-1")
        .header("x-git-username", "u")
        .header("x-git-token", "p")
        .header("x-max-staleness", "0")
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(
        count(&after),
        0,
        "a commit whose branch is gone must stop being enumerated: {after}"
    );
    Ok(())
}

#[tokio::test]
async fn a_fetch_that_changed_nothing_keeps_live_page_tokens_valid() -> R {
    let server = spawn_server("noopfetch", TOKEN)?;
    wait_healthy(server.port).await?;
    let repo = fixture_origin(&server.dir)?;
    get_json(server.port, "/v1/commits", &repo).await?;

    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{}", server.port);
    let page = |token: Option<String>, staleness: &'static str| {
        let (client, base, repo) = (client.clone(), base.clone(), repo.clone());
        async move {
            let mut params = vec![
                ("repo".to_owned(), repo),
                ("page_size".to_owned(), "1".to_owned()),
            ];
            if let Some(token) = token {
                params.push(("page_token".to_owned(), token));
            }
            client
                .get(format!("{base}/v1/commits"))
                .query(&params)
                .bearer_auth(TOKEN)
                .header("x-tenant-id", "tenant-1")
                .header("x-source-id", "source-1")
                .header("x-git-username", "u")
                .header("x-git-token", "p")
                .header("x-max-staleness", staleness)
                .send()
                .await
        }
    };

    let first: serde_json::Value = page(None, "300").await?.json().await?;
    let token = first["next_page_token"]
        .as_str()
        .ok_or("a truncated page must carry a cursor")?
        .to_owned();

    // Force a refresh against an origin that has not moved — what a second
    // stream of the same sync does routinely once the window lapses.
    let _ = page(None, "0").await?;

    let continued = page(Some(token), "300").await?;
    assert_eq!(
        continued.status(),
        200,
        "an unchanged origin must not invalidate a cursor mid-walk"
    );
    Ok(())
}

#[tokio::test]
async fn file_changes_cover_every_branch_not_just_the_default() -> R {
    // The CDK gitlab connector collects commits for all branches but file
    // changes only for the default branch's head. The proxy must not inherit
    // that: a side-branch commit has file rows like any other.
    let server = spawn_server("branchfiles", TOKEN)?;
    wait_healthy(server.port).await?;
    let repo = branching_origin(&server.dir)?;

    let commits = get_json(server.port, "/v1/commits", &repo).await?;
    let side = commits["items"]
        .as_array()
        .ok_or("no items")?
        .iter()
        .find(|row| row["message"] == "on the side branch")
        .ok_or("the side-branch commit must be enumerated")?;
    let side_sha = side["sha"].as_str().ok_or("no sha")?.to_owned();
    assert_eq!(side["is_in_default_branch"], false);

    let changes = get_json(server.port, "/v1/file-changes", &repo).await?;
    let rows: Vec<&serde_json::Value> = changes["items"]
        .as_array()
        .ok_or("no items")?
        .iter()
        .filter(|row| row["sha"] == side_sha.as_str())
        .collect();

    assert!(
        !rows.is_empty(),
        "a commit off the default branch must still yield file rows: {changes}"
    );
    assert_eq!(rows[0]["filename"], "b.txt");
    Ok(())
}

/// Two commits whose TEXT order is the reverse of their chronological order:
/// `%cI` keeps the committer's offset, so `10:00+02:00` (08:00Z) renders after
/// `09:00Z` as text while being an hour earlier.
fn mixed_timezone_origin(root: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let origin = root.join("timezones");
    std::fs::create_dir_all(&origin)?;
    let script = "git init -q -b main . && \
         git config uploadpack.allowFilter true && \
         git config uploadpack.allowAnySHA1InWant true && \
         echo a > a.txt && git add a.txt && \
         GIT_AUTHOR_DATE='2026-08-01T09:00:00+0000' GIT_COMMITTER_DATE='2026-08-01T09:00:00+0000' \
           git commit -qm 'utc nine' && \
         echo b > b.txt && git add b.txt && \
         GIT_AUTHOR_DATE='2026-08-01T10:00:00+0200' GIT_COMMITTER_DATE='2026-08-01T10:00:00+0200' \
           git commit -qm 'berlin ten'";
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

#[tokio::test]
async fn an_interrupted_sync_resumes_without_losing_a_commit() -> R {
    let server = spawn_server("resume", TOKEN)?;
    wait_healthy(server.port).await?;
    let repo = mixed_timezone_origin(&server.dir)?;
    get_json(server.port, "/v1/commits", &repo).await?;

    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{}", server.port);
    let ask = |params: Vec<(String, String)>| {
        let (client, base) = (client.clone(), base.clone());
        async move {
            client
                .get(format!("{base}/v1/commits"))
                .query(&params)
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

    // One page, then the sync dies — exactly what a 429 or a restart does.
    let first = ask(vec![
        ("repo".to_owned(), repo.clone()),
        ("page_size".to_owned(), "1".to_owned()),
    ])
    .await?;
    let row = &first["items"][0];
    let checkpoint = row["committed_date"]
        .as_str()
        .ok_or("a row must carry a committed_date")?
        .to_owned();
    let mut seen = vec![
        row["message"]
            .as_str()
            .ok_or("a row must carry a message")?
            .to_owned(),
    ];

    // The connector resumes from the cursor it checkpointed.
    let resumed = ask(vec![
        ("repo".to_owned(), repo.clone()),
        ("since".to_owned(), checkpoint.clone()),
    ])
    .await?;
    for item in resumed["items"].as_array().ok_or("no items")? {
        let message = item["message"].as_str().ok_or("no message")?.to_owned();
        if !seen.contains(&message) {
            seen.push(message);
        }
    }

    seen.sort();
    assert_eq!(
        seen,
        vec!["berlin ten".to_owned(), "utc nine".to_owned()],
        "a resumed sync must not skip a commit whose offset makes it sort          later than it happened (checkpoint was {checkpoint})"
    );
    Ok(())
}

#[tokio::test]
async fn a_page_token_from_another_repository_is_refused() -> R {
    let server = spawn_server("crossrepo", TOKEN)?;
    wait_healthy(server.port).await?;
    let mine = fixture_origin(&server.dir)?;
    let theirs = skewed_origin(&server.dir, "other")?;

    get_json(server.port, "/v1/commits", &mine).await?;
    get_json(server.port, "/v1/commits", &theirs).await?;

    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{}", server.port);
    let first: serde_json::Value = client
        .get(format!("{base}/v1/commits"))
        .query(&[("repo", mine.as_str()), ("page_size", "1")])
        .bearer_auth(TOKEN)
        .header("x-tenant-id", "tenant-1")
        .header("x-source-id", "source-1")
        .header("x-git-username", "u")
        .header("x-git-token", "p")
        .send()
        .await?
        .json()
        .await?;
    let token = first["next_page_token"]
        .as_str()
        .ok_or("a truncated page must carry a cursor")?;

    // Both repositories are warm at generation 1, so nothing but the entry
    // binding stands between this token and the wrong repository's history.
    let replayed = client
        .get(format!("{base}/v1/commits"))
        .query(&[
            ("repo", theirs.as_str()),
            ("page_size", "1"),
            ("page_token", token),
        ])
        .bearer_auth(TOKEN)
        .header("x-tenant-id", "tenant-1")
        .header("x-source-id", "source-1")
        .header("x-git-username", "u")
        .header("x-git-token", "p")
        .send()
        .await?;
    assert_eq!(
        replayed.status(),
        400,
        "a cursor must not continue a repository it was not minted for"
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
async fn branches_honour_page_size_and_paginate_by_name() -> R {
    let server = spawn_server("branchpages", TOKEN)?;
    wait_healthy(server.port).await?;
    let repo = fixture_origin(&server.dir)?;

    // Six branches: enough that an unpaginated response is visibly wrong.
    let script = ["b1", "b2", "b3", "b4", "b5"]
        .map(|name| format!("git branch {name}"))
        .join(" && ");
    let output = Command::new("sh")
        .arg("-ec")
        .arg(&script)
        .current_dir(server.dir.join("origin"))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    get_json(server.port, "/v1/branches", &repo).await?;

    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{}", server.port);
    let page = |token: Option<String>| {
        let client = client.clone();
        let base = base.clone();
        let repo = repo.clone();
        async move {
            let mut request = client
                .get(format!("{base}/v1/branches"))
                .query(&[("repo", repo.as_str()), ("page_size", "2")])
                .bearer_auth(TOKEN)
                .header("x-tenant-id", "tenant-1")
                .header("x-source-id", "source-1")
                .header("x-git-username", "u")
                .header("x-git-token", "p");
            if let Some(token) = token {
                request = request.query(&[("page_token", token)]);
            }
            request.send().await?.json::<serde_json::Value>().await
        }
    };

    let first = page(None).await?;
    let items = first["items"].as_array().ok_or("no items")?;
    assert_eq!(items.len(), 2, "page_size must bound the branch response");

    let token = first["next_page_token"]
        .as_str()
        .ok_or("a truncated branch page must carry a cursor")?
        .to_owned();
    let second = page(Some(token)).await?;
    let next = second["items"].as_array().ok_or("no items")?;
    assert_eq!(next.len(), 2);

    let first_names: Vec<&str> = items.iter().filter_map(|b| b["name"].as_str()).collect();
    let next_names: Vec<&str> = next.iter().filter_map(|b| b["name"].as_str()).collect();
    assert!(
        first_names.iter().all(|n| !next_names.contains(n)),
        "pages must not overlap: {first_names:?} vs {next_names:?}"
    );
    assert!(
        first_names.last() < next_names.first(),
        "branches walk in ascending name order: {first_names:?} then {next_names:?}"
    );
    Ok(())
}

#[tokio::test]
async fn refuses_to_boot_on_empty_required_config() -> R {
    let port = free_port()?;
    let dir = test_dir("invalid")?;
    let config = write_config(&dir, port, "", false)?;

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

#[tokio::test]
async fn a_file_row_carries_the_name_git_has_on_disk() -> R {
    let server = spawn_server("awkward", TOKEN)?;
    wait_healthy(server.port).await?;
    let repo = awkward_paths_origin(&server.dir)?;

    let changes = get_json(server.port, "/v1/file-changes", &repo).await?;
    let rows = changes["items"].as_array().ok_or("no items")?;

    let names: Vec<&str> = rows
        .iter()
        .filter_map(|row| row["filename"].as_str())
        .collect();
    let expected = [
        "dir with space/a b.txt",
        "quote\".txt",
        "back\\slash.txt",
        "unicode-ä.txt",
        "has b/nested.txt",
        "tab\there.txt",
    ];
    for name in expected {
        assert!(
            names.contains(&name),
            "the literal path must reach the row, not its escaped spelling: \
             {name:?} missing from {names:?}"
        );
    }

    // The patch text is keyed by the same name, or it silently detaches from
    // the row it belongs to.
    for row in rows {
        let name = row["filename"].as_str().unwrap_or_default();
        assert!(
            row["patch"].is_string(),
            "{name:?} must carry its own diff: {row}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_submodule_does_not_force_a_repository_out_of_the_blobless_cache() -> R {
    // The gitlink id belongs to the submodule. Requesting it from the
    // superproject's origin fails with "not our ref", which this service reads
    // as an origin refusing promisor wants — so a single submodule used to
    // promote the whole repository to a full clone, permanently.
    let server = spawn_server("submodule", TOKEN)?;
    wait_healthy(server.port).await?;
    let repo = submodule_origin(&server.dir)?;

    let changes = get_json(server.port, "/v1/file-changes", &repo).await?;
    let rows = changes["items"].as_array().ok_or("no items")?;
    let names: Vec<&str> = rows
        .iter()
        .filter_map(|row| row["filename"].as_str())
        .collect();
    assert!(
        names.contains(&"o.txt"),
        "the ordinary file must still be served: {names:?}"
    );

    let commits = get_json(server.port, "/v1/commits", &repo).await?;
    assert!(
        commits["items"]
            .as_array()
            .is_some_and(|items| !items.is_empty()),
        "commits must be served for a repository holding a submodule"
    );
    Ok(())
}

#[tokio::test]
async fn a_copied_file_reports_the_status_the_contract_lists() -> R {
    // DESIGN §4.2 lists `copied` and says collapsing it into `modified` would
    // misreport it — but the invocation passed `-M` alone, under which git
    // never emits a `C` status and the row came back as `added`.
    let server = spawn_server("copying", TOKEN)?;
    wait_healthy(server.port).await?;
    let repo = copying_origin(&server.dir)?;

    let changes = get_json(server.port, "/v1/file-changes", &repo).await?;
    let rows = changes["items"].as_array().ok_or("no items")?;
    let copied = rows
        .iter()
        .find(|row| row["filename"] == "b.txt")
        .ok_or("b.txt must have a row")?;

    assert_eq!(
        copied["status"], "copied",
        "a verbatim copy must be reported as such: {copied}"
    );
    assert_eq!(
        copied["previous_filename"], "a.txt",
        "and must name the file it came from"
    );
    Ok(())
}

#[tokio::test]
async fn a_deleted_index_changes_nothing_a_caller_can_see() -> R {
    // The index is a cache of the two whole-history walks, and the walks stay
    // as the fallback. Deleting it mid-flight must leave the response
    // byte-identical — otherwise a failed index build would silently change
    // what bronze receives.
    let server = spawn_server("index-fallback", TOKEN)?;
    wait_healthy(server.port).await?;
    let repo = fixture_origin(&server.dir)?;

    let indexed_commits = get_json(server.port, "/v1/commits", &repo).await?;
    let indexed_changes = get_json(server.port, "/v1/file-changes", &repo).await?;

    let mut removed = 0;
    for entry in std::fs::read_dir(server.dir.join("data").join("repos"))? {
        let info = entry?.path().join("repo.git").join("info");
        let Ok(files) = std::fs::read_dir(&info) else {
            continue;
        };
        for file in files.flatten() {
            if file
                .file_name()
                .to_string_lossy()
                .starts_with("page-index-")
            {
                std::fs::remove_file(file.path())?;
                removed += 1;
            }
        }
    }
    assert!(removed > 0, "the clone must have built an index to delete");

    let walked_commits = get_json(server.port, "/v1/commits", &repo).await?;
    let walked_changes = get_json(server.port, "/v1/file-changes", &repo).await?;
    assert_eq!(
        indexed_commits, walked_commits,
        "commits must not depend on the index"
    );
    assert_eq!(
        indexed_changes, walked_changes,
        "file changes must not depend on the index"
    );
    Ok(())
}

#[tokio::test]
async fn the_index_is_what_a_page_actually_reads() -> R {
    // The fallback test proves deleting the index changes nothing; this one
    // proves the index is not dead weight. A row removed from a VALID index
    // must disappear from the response — if it did not, every page would be
    // silently paying the whole-history walk the index exists to remove.
    let server = spawn_server("index-live", TOKEN)?;
    wait_healthy(server.port).await?;
    let repo = fixture_origin(&server.dir)?;

    let before = get_json(server.port, "/v1/commits", &repo).await?;
    let full = before["items"].as_array().ok_or("no items")?.len();
    assert!(full >= 2, "the fixture must have at least two commits");

    let mut clipped = 0;
    for entry in std::fs::read_dir(server.dir.join("data").join("repos"))? {
        let info = entry?.path().join("repo.git").join("info");
        let Ok(files) = std::fs::read_dir(&info) else {
            continue;
        };
        for file in files.flatten() {
            if !file
                .file_name()
                .to_string_lossy()
                .starts_with("page-index-")
            {
                continue;
            }
            // Drop one ROW and restate the trailer: the count is what lets
            // the reader tell a deliberate edit from crash truncation.
            let text = std::fs::read_to_string(file.path())?;
            let mut lines: Vec<&str> = text.lines().collect();
            let trailer = lines.pop().ok_or("index must have a trailer")?;
            let (tag, count) = trailer.split_once('\u{1f}').ok_or("malformed trailer")?;
            assert_eq!(tag, "count", "the last line must be the count trailer");
            lines.pop();
            let restated = format!("count\u{1f}{}", count.parse::<usize>()? - 1);
            lines.push(&restated);
            std::fs::write(file.path(), lines.join("\n") + "\n")?;
            clipped += 1;
        }
    }
    assert!(clipped > 0, "there must have been an index to clip");

    let after = get_json(server.port, "/v1/commits", &repo).await?;
    assert_eq!(
        after["items"].as_array().ok_or("no items")?.len(),
        full - 1,
        "a clipped index must clip the response, or the index is not being read"
    );

    // Crash truncation — a dropped tail with a now-wrong trailer — must NOT
    // clip the response: the reader detects it, drops the file, and the live
    // walk serves the full history.
    for entry in std::fs::read_dir(server.dir.join("data").join("repos"))? {
        let info = entry?.path().join("repo.git").join("info");
        let Ok(files) = std::fs::read_dir(&info) else {
            continue;
        };
        for file in files.flatten() {
            if !file
                .file_name()
                .to_string_lossy()
                .starts_with("page-index-")
            {
                continue;
            }
            let text = std::fs::read_to_string(file.path())?;
            let mut lines: Vec<&str> = text.lines().collect();
            lines.pop();
            std::fs::write(file.path(), lines.join("\n") + "\n")?;
        }
    }
    let recovered = get_json(server.port, "/v1/commits", &repo).await?;
    assert_eq!(
        recovered["items"].as_array().ok_or("no items")?.len(),
        full,
        "a truncated index must fall back to the live walk, never serve short history"
    );
    Ok(())
}
