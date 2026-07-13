//! CLI-level tests: drive the built `routegen` binary end to end (output mode,
//! the CI `--check` up-to-date assertion, and the failure paths).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_routegen");

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("routegen-cli-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn routegen(args: &[&str]) -> std::process::Output {
    Command::new(BIN).args(args).output().expect("run routegen")
}

#[test]
fn writes_a_valid_config_to_output() {
    let routes = fixtures().join("full.routes.yaml");
    let out = tmp("out.conf");
    let o = routegen(&[
        "--routes",
        routes.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--set-real-ip-from",
        "10.0.0.0/8",
    ]);
    assert!(
        o.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let conf = fs::read_to_string(&out).unwrap();
    assert!(conf.contains("resolver local=on ipv6=off;"));
    assert!(conf.contains("set_real_ip_from 10.0.0.0/8;"));
    assert!(conf.contains("access_by_lua_block"));
}

#[test]
fn check_passes_when_in_sync() {
    let routes = fixtures().join("full.routes.yaml");
    let out = tmp("insync.conf");
    // Generate, then assert --check against the same output is clean.
    assert!(
        routegen(&[
            "--routes",
            routes.to_str().unwrap(),
            "-o",
            out.to_str().unwrap()
        ])
        .status
        .success()
    );
    let o = routegen(&[
        "--routes",
        routes.to_str().unwrap(),
        "--check",
        out.to_str().unwrap(),
    ]);
    assert!(
        o.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn check_fails_on_drift() {
    let routes = fixtures().join("full.routes.yaml");
    let stale = tmp("stale.conf");
    fs::write(&stale, "# stale, not what routegen would emit\n").unwrap();
    let o = routegen(&[
        "--routes",
        routes.to_str().unwrap(),
        "--check",
        stale.to_str().unwrap(),
    ]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("out of date"));
}

#[test]
fn fails_on_invalid_routes() {
    let bad = tmp("bad.routes.yaml");
    fs::write(
        &bad,
        "version: 1\nroutes:\n- {prefix: /nope, upstream: 'http://h:1'}\n",
    )
    .unwrap();
    let o = routegen(&["--routes", bad.to_str().unwrap()]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("/api/"));
}

#[test]
fn fails_on_missing_routes_file() {
    let o = routegen(&["--routes", "/no/such/routes.yaml"]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("read routes file"));
}
